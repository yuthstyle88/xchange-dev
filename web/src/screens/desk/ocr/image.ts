// Browser canvas rotation and scaling for passport photos (design recognize-passport-read.md §4.2, #542 §7).
import { PassportReadError } from './passport-read';

export type Angle = 0 | 90 | 180 | 270;

const MAX_SIDE_PX = 2600;
const MAX_IMAGE_BYTES = 8 * 1024 * 1024;
const ACCEPTED_IMAGE_TYPES = new Set(['image/jpeg', 'image/png', 'image/webp']);
const JPEG_QUALITIES = [0.88, 0.75, 0.6, 0.45];

function decodeImage(file: Blob): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    if (typeof URL === 'undefined' || typeof URL.createObjectURL !== 'function') {
      reject(new PassportReadError('failed'));
      return;
    }
    const url = URL.createObjectURL(file);
    const img = new Image();
    img.onload = () => {
      URL.revokeObjectURL(url);
      resolve(img);
    };
    img.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new PassportReadError('imageInvalid'));
    };
    img.src = url;
  });
}

function encodeJpeg(canvas: HTMLCanvasElement, quality: number): Promise<Blob | null> {
  return new Promise((resolve) => {
    if (typeof canvas.toBlob !== 'function') {
      resolve(null);
      return;
    }
    canvas.toBlob((blob) => resolve(blob), 'image/jpeg', quality);
  });
}

export async function prepareImage(file: Blob, angle: Angle): Promise<Blob> {
  const img = await decodeImage(file);
  const srcW = img.naturalWidth || img.width;
  const srcH = img.naturalHeight || img.height;
  if (srcW <= 0 || srcH <= 0) {
    throw new PassportReadError('imageInvalid');
  }

  const longer = Math.max(srcW, srcH);
  if (angle === 0 && longer <= MAX_SIDE_PX && file.size > 0 && file.size <= MAX_IMAGE_BYTES && ACCEPTED_IMAGE_TYPES.has(file.type)) {
    return file;
  }

  const scale = longer > MAX_SIDE_PX ? MAX_SIDE_PX / longer : 1;
  const drawW = Math.max(1, Math.round(srcW * scale));
  const drawH = Math.max(1, Math.round(srcH * scale));

  const canvas = document.createElement('canvas');
  if (angle === 90 || angle === 270) {
    canvas.width = drawH;
    canvas.height = drawW;
  } else {
    canvas.width = drawW;
    canvas.height = drawH;
  }

  let ctx: CanvasRenderingContext2D | null = null;
  try {
    ctx = canvas.getContext('2d');
  } catch {
    ctx = null;
  }
  if (!ctx) {
    throw new PassportReadError('failed');
  }

  ctx.translate(canvas.width / 2, canvas.height / 2);
  if (angle !== 0) {
    ctx.rotate((angle * Math.PI) / 180);
  }
  ctx.drawImage(img, -drawW / 2, -drawH / 2, drawW, drawH);

  let lastBlob: Blob | null = null;
  for (const q of JPEG_QUALITIES) {
    const blob = await encodeJpeg(canvas, q);
    if (!blob) break;
    lastBlob = blob;
    if (blob.size <= MAX_IMAGE_BYTES) {
      return blob;
    }
  }
  if (!lastBlob) {
    throw new PassportReadError('failed');
  }
  if (lastBlob.size > MAX_IMAGE_BYTES) {
    throw new PassportReadError('imageTooLarge');
  }
  return lastBlob;
}
