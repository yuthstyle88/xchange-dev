// A small stand-in for a v2 bundle, written for `extract-v2-pages.test.mjs`.
// It has the shape the script looks for — three pages as template literals in
// `var <name>_html_default` declarations — and none of v2's content.

// a string that only looks like a declaration, before the real one
var decoy = "var admin_html_default = `<p>not this one</p>`;";

// src/online/lib/admin-html.ts
var admin_html_default = `<!doctype html>
<html lang="th">
<head><title>XCHANGE \xB7 แอดมิน (bundle fixture)</title></head>
<body>
<script>const tick = "\`", dollar = "\${not.interpolated}", slash = "a\\\\b", nl = "x\\ny", u = "ก\u{1F600}";</script>
</body>
</html>
`;
function htmlResponse(request, html) {
  return html;
}
function GET(request) {
  return htmlResponse(request, admin_html_default);
}

// src/online/lib/index-html.ts
var index_html_default = `<!doctype html><html><head><title>desk (bundle fixture)</title></head><body><a href="/documents">d</a></body></html>`;
function GET2(request) {
  return htmlResponse(request, index_html_default);
}

// src/online/lib/receipt-html.ts
var receipt_html_default = `<!doctype html>
<html lang="en"><head><title>ticket • sample (bundle fixture)</title></head><body>&amp; \$1</body></html>`;
function GET3(request) {
  return htmlResponse(request, receipt_html_default);
}
var other_html_default = `<p>a fourth page the script has no name for</p>`;
export { GET, GET2, GET3, decoy, other_html_default };
