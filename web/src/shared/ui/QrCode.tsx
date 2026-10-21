import { Show, type JSX } from 'solid-js';
import qrcode from 'qrcode-generator';

export interface QrCodeProps {
  value: string;
  label: string;
  downloadName?: string;
  downloadLabel?: string;
}

export function QrCode(props: QrCodeProps): JSX.Element {
  let imgEl: HTMLDivElement | undefined;

  const svgHtml = () => {
    const fn = typeof qrcode === 'function' ? qrcode : (qrcode as unknown as { default: typeof qrcode }).default;
    const qr = fn(0, 'M');
    qr.addData(props.value);
    qr.make();
    return qr.createSvgTag({ scalable: true });
  };

  const handleDownload = () => {
    const name = props.downloadName;
    if (!name) return;
    const svg = imgEl?.querySelector('svg');
    if (!svg) return;
    const clone = svg.cloneNode(true) as SVGSVGElement;
    clone.setAttribute('width', '512');
    clone.setAttribute('height', '512');
    const xml = new XMLSerializer().serializeToString(clone);
    const canvas = document.createElement('canvas');
    canvas.width = 512;
    canvas.height = 512;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    ctx.fillStyle = '#ffffff';
    ctx.fillRect(0, 0, canvas.width, canvas.height);

    const img = new Image();
    img.onload = () => {
      ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
      canvas.toBlob((blob) => {
        if (!blob) return;
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = `${name}.png`;
        a.click();
        setTimeout(() => URL.revokeObjectURL?.(url), 1000);
      }, 'image/png');
    };
    img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(xml)}`;
  };

  return (
    <>
      <div
        ref={imgEl}
        role="img"
        aria-label={props.label}
        innerHTML={svgHtml()}
      />
      <Show when={props.downloadName}>
        <button
          type="button"
          class="xc-qr-download"
          data-action="download-qr"
          onClick={handleDownload}
        >
          {props.downloadLabel}
        </button>
      </Show>
    </>
  );
}

export default QrCode;
