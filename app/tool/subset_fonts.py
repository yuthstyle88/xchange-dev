#!/usr/bin/env python3
"""
Subset CJK fallback fonts for XCHANGE kiosk.

Reads app/assets/l10n/kiosk.json, collects characters for zh, zh-HK, ja, and ko,
adds ASCII, full-width digits/letters, and common CJK punctuation, and uses
pyftsubset (fonttools) to generate compact TTFs for app/assets/fonts/.

Records metadata in app/assets/fonts/SUBSET.txt.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

try:
    import fontTools
    from fontTools.ttLib import TTFont
except ImportError:
    sys.exit("Error: fonttools is required. Install with: pip install --user fonttools")

# Upstream font sources from Google Fonts (SIL OFL 1.1)
FONT_SOURCES = {
    "NotoSansSC.ttf": {
        "lang": "zh",
        "url": "https://github.com/google/fonts/raw/main/ofl/notosanssc/NotoSansSC%5Bwght%5D.ttf",
    },
    "NotoSansTC.ttf": {
        "lang": "zh-HK",
        "url": "https://github.com/google/fonts/raw/main/ofl/notosanstc/NotoSansTC%5Bwght%5D.ttf",
    },
    "NotoSansJP.ttf": {
        "lang": "ja",
        "url": "https://github.com/google/fonts/raw/main/ofl/notosansjp/NotoSansJP%5Bwght%5D.ttf",
    },
    "NotoSansKR.ttf": {
        "lang": "ko",
        "url": "https://github.com/google/fonts/raw/main/ofl/notosanskr/NotoSansKR%5Bwght%5D.ttf",
    },
}

CACHE_DIR = Path.home() / ".cache" / "noto_fonts_full"


def find_app_dir() -> Path:
    """Find the app/ directory from current working dir or script location."""
    candidates = [
        Path.cwd() / "app",
        Path.cwd(),
        Path(__file__).resolve().parent.parent,
    ]
    for c in candidates:
        if (c / "assets" / "l10n" / "kiosk.json").exists():
            return c
    sys.exit("Error: Could not locate app/assets/l10n/kiosk.json")


def ensure_source_font(filename: str, source_info: dict, source_dir: Path | None) -> Path:
    """Ensure the full source font is available locally."""
    # 1. User-specified source dir
    if source_dir and (source_dir / filename).exists():
        return source_dir / filename

    # 2. Local user cache
    CACHE_DIR.mkdir(parents=True, exist_ok=True)
    cached_path = CACHE_DIR / filename
    if cached_path.exists() and cached_path.stat().st_size > 1_000_000:
        return cached_path

    # 3. Download from upstream
    print(f"Downloading {filename} from upstream...")
    urllib.request.urlretrieve(source_info["url"], cached_path)
    return cached_path


def build_common_chars() -> set[str]:
    """Base character set included in all CJK subsets: ASCII, punctuation, symbols."""
    chars = set()
    # ASCII printable (0x20 - 0x7E)
    chars.update(chr(c) for c in range(0x20, 0x7F))
    # General Punctuation (0x2000 - 0x206F)
    chars.update(chr(c) for c in range(0x2000, 0x2070))
    # Common CJK Symbols and Punctuation (0x3000 - 0x303F)
    chars.update(chr(c) for c in range(0x3000, 0x3040))
    # Fullwidth ASCII variants and forms (0xFF01 - 0xFF5E)
    chars.update(chr(c) for c in range(0xFF01, 0xFF5F))
    # Additional fullwidth currency and punctuation (0xFFE0 - 0xFFEE)
    chars.update(chr(c) for c in range(0xFFE0, 0xFFEF))
    # Common currency & UI symbols
    chars.update(["✓", "・", "¥", "￥", "₩", "฿", "€", "£", "$", "…", "—", "–", "•"])
    return chars


def main():
    app_dir = find_app_dir()
    l10n_file = app_dir / "assets" / "l10n" / "kiosk.json"
    fonts_dir = app_dir / "assets" / "fonts"
    fonts_dir.mkdir(parents=True, exist_ok=True)

    with open(l10n_file, "r", encoding="utf-8") as f:
        kiosk_data = json.load(f)

    # Collect language names and search keywords from all declared langs
    lang_chars = set()
    for item in kiosk_data.get("langs", []):
        lang_chars.update(item.get("name", ""))
        lang_chars.update(item.get("search", ""))

    common_chars = build_common_chars()
    common_chars.update(lang_chars)

    # Load CJK browser currency names
    cjk_currencies_file = app_dir / "tool" / "cjk_currency_names.json"
    cjk_currencies = {}
    chrome_version = "Unknown"
    if cjk_currencies_file.exists():
        with open(cjk_currencies_file, "r", encoding="utf-8") as f:
            cjk_curr_data = json.load(f)
            chrome_version = cjk_curr_data.get("chromeVersion", "Unknown")
            cjk_currencies = cjk_curr_data.get("currencies", {})

    pyftsubset = shutil.which("pyftsubset")
    if not pyftsubset:
        # Check standard user bin
        user_bin = Path.home() / ".local" / "bin" / "pyftsubset"
        if user_bin.exists():
            pyftsubset = str(user_bin)
        else:
            sys.exit("Error: pyftsubset executable not found in PATH")

    subset_records = []

    for filename, info in FONT_SOURCES.items():
        lang_code = info["lang"]
        source_font_path = ensure_source_font(filename, info, None)

        # Get source font version from name table (nameID 5 = Version)
        source_tt = TTFont(source_font_path)
        source_version = source_tt["name"].getDebugName(5) or "Unknown"

        # Collect characters for this language:
        # 1. Base characters (ASCII, punctuation, symbols) + all language names & search terms
        target_chars = set(common_chars)

        # 2. Text sentences for this language from kiosk.json
        texts = kiosk_data.get("text", {}).get(lang_code, {})
        for s in texts.values():
            target_chars.update(s)

        # 3. Currency names for this language from cjk_currency_names.json
        curr_names = cjk_currencies.get(lang_code, {}).get("names", {})
        for name in curr_names.values():
            target_chars.update(name)

        out_path = fonts_dir / filename

        with tempfile.NamedTemporaryFile("w", encoding="utf-8", delete=False) as tf:
            tf.write("".join(sorted(target_chars)))
            temp_text_path = tf.name

        try:
            cmd = [
                pyftsubset,
                str(source_font_path),
                f"--text-file={temp_text_path}",
                f"--output-file={out_path}",
                "--layout-features=*",
            ]
            res = subprocess.run(cmd, capture_output=True, text=True)
            if res.returncode != 0:
                sys.exit(f"Error running pyftsubset for {filename}:\n{res.stderr}")
        finally:
            if os.path.exists(temp_text_path):
                os.remove(temp_text_path)

        # Inspect resulting subset font
        subset_tt = TTFont(out_path)
        cmap = subset_tt.getBestCmap() or {}
        char_count = len(cmap)
        file_size_kb = out_path.stat().st_size / 1024

        subset_records.append({
            "font": filename,
            "lang": lang_code,
            "source_version": source_version,
            "char_count": char_count,
            "size_kb": file_size_kb,
        })
        print(f"Generated {filename}: {char_count} glyphs ({file_size_kb:.1f} KB)")

    # Write SUBSET.txt
    subset_txt_path = fonts_dir / "SUBSET.txt"
    lines = [
        f"Subset generated by app/tool/subset_fonts.py",
        f"FontTools version: {fontTools.__version__}",
        f"Chrome version for currency names: {chrome_version}",
        "",
        "Subset CJK Fonts (includes kiosk.json texts, language names, common punctuation/ASCII, and cjk_currency_names.json):",
    ]
    for r in subset_records:
        lines.append(f"- {r['font']} ({r['lang']}):")
        lines.append(f"  Source version: {r['source_version']}")
        lines.append(f"  Characters: {r['char_count']}")
        lines.append(f"  File size: {r['size_kb']:.1f} KB")

    content = "\n".join(lines) + "\n"
    subset_txt_path.write_text(content, encoding="utf-8")
    print(f"\nWrote metadata to {subset_txt_path}")


if __name__ == "__main__":
    main()
