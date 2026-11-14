// Images in a chat message: reading attached files, thumbnails, and a larger view on click.

import { useState } from "react";
import { X } from "lucide-react";
import type { ImageInput } from "../../domain/api";
import { useFocusTrap } from "../../ui/useFocusTrap";
import type { TranscriptImage } from "./types";

/** Mirrors the backend's checks (`chats::check_images`). */
export const IMAGE_TYPES = ["image/png", "image/jpeg", "image/gif", "image/webp"];
export const IMAGE_MAX_BYTES = 5 * 1024 * 1024;
export const IMAGES_MAX = 10;

/** Reads image files as attachments; files that can't be attached come back as `errors`. */
export async function readImages(files: File[], already: number): Promise<{ images: ImageInput[]; errors: string[] }> {
  const images: ImageInput[] = [];
  const errors: string[] = [];
  for (const f of files) {
    if (!IMAGE_TYPES.includes(f.type)) errors.push(`${f.name || "That file"} isn't a PNG, JPEG, GIF or WebP.`);
    else if (f.size > IMAGE_MAX_BYTES) errors.push(`${f.name || "That image"} is over 5 MB.`);
    else if (already + images.length >= IMAGES_MAX) errors.push(`At most ${IMAGES_MAX} images per message.`);
    else {
      const url = await new Promise<string>((resolve, reject) => {
        const r = new FileReader();
        r.onload = () => resolve(String(r.result));
        r.onerror = () => reject(r.error);
        r.readAsDataURL(f);
      });
      images.push({ mediaType: f.type, dataBase64: url.slice(url.indexOf(",") + 1) });
    }
  }
  return { images, errors: [...new Set(errors)] };
}

export const toTranscriptImage = (i: ImageInput): TranscriptImage => ({
  mediaType: i.mediaType,
  data: i.dataBase64,
  bytes: Math.floor((i.dataBase64.length * 3) / 4),
});

const sizeLabel = (bytes: number) => (bytes >= 1024 * 1024 ? `${(bytes / 1024 / 1024).toFixed(1)} MB` : `${Math.ceil(bytes / 1024)} KB`);

/** Thumbnails; each opens larger on click. `onRemove` adds a remove button to each. */
export function ImageThumbs({ images, onRemove }: { images: TranscriptImage[]; onRemove?: (i: number) => void }) {
  const [open, setOpen] = useState<number | null>(null);
  if (!images.length) return null;
  const shown = open !== null ? images[open] : undefined;
  return (
    <div className="chat-images">
      {images.map((img, i) => {
        const label = `Image ${i + 1}`;
        return (
          <div key={i} className="chat-image">
            {img.data ? (
              <button type="button" className="chat-image-btn" aria-label={`View ${label.toLowerCase()} larger`} onClick={() => setOpen(i)}>
                <img src={`data:${img.mediaType};base64,${img.data}`} alt={label} />
              </button>
            ) : (
              <span className="chat-image-big" title="Too big to show in the history">
                Image · {sizeLabel(img.bytes)}
              </span>
            )}
            {onRemove && (
              <button type="button" className="chat-image-remove" aria-label={`Remove ${label.toLowerCase()}`} title="Remove" onClick={() => onRemove(i)}>
                <X size={11} strokeWidth={2.5} aria-hidden />
              </button>
            )}
          </div>
        );
      })}
      {shown?.data && <ImageViewer src={`data:${shown.mediaType};base64,${shown.data}`} onClose={() => setOpen(null)} />}
    </div>
  );
}

function ImageViewer({ src, onClose }: { src: string; onClose: () => void }) {
  const ref = useFocusTrap<HTMLDivElement>(onClose);
  return (
    <>
      <div className="confirm-scrim" onClick={onClose} aria-hidden />
      <div ref={ref} className="chat-image-viewer" role="dialog" aria-modal="true" aria-label="Image" tabIndex={-1} onClick={onClose}>
        <img src={src} alt="Attached image" />
        <button type="button" className="chat-image-close" aria-label="Close" data-autofocus onClick={onClose}>
          <X size={14} strokeWidth={2} aria-hidden />
        </button>
      </div>
    </>
  );
}
