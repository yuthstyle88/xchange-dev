import type { JSX } from 'solid-js';
import qrcode from 'qrcode-generator';

export interface QrCodeProps {
  value: string;
  label: string;
}

export function QrCode(props: QrCodeProps): JSX.Element {
  const svgHtml = () => {
    const fn = typeof qrcode === 'function' ? qrcode : (qrcode as unknown as { default: typeof qrcode }).default;
    const qr = fn(0, 'M');
    qr.addData(props.value);
    qr.make();
    return qr.createSvgTag({ scalable: true });
  };

  return (
    <div
      role="img"
      aria-label={props.label}
      innerHTML={svgHtml()}
    />
  );
}

export default QrCode;
