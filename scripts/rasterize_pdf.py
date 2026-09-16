# Rasterize a scanned PDF to JPEG page images. Local only.
# Uses pypdfium2 and Pillow already present on this machine.

import sys
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 3:
        sys.stderr.write("usage: rasterize_pdf.py <pdf> <out_dir>\n")
        return 2
    pdf_path = Path(sys.argv[1])
    out_dir = Path(sys.argv[2])
    if not pdf_path.is_file():
        sys.stderr.write("pdf missing: %s\n" % pdf_path)
        return 1
    out_dir.mkdir(parents=True, exist_ok=True)
    import pypdfium2 as pdfium

    pdf = pdfium.PdfDocument(str(pdf_path))
    scale = 150.0 / 72.0
    count = len(pdf)
    for index in range(count):
        page = pdf[index]
        bitmap = page.render(scale=scale)
        image = bitmap.to_pil().convert("RGB")
        dest = out_dir / ("page-%03d.jpg" % (index + 1))
        image.save(dest, "JPEG", quality=75)
        page.close()
        print(dest)
    pdf.close()
    if count == 0:
        sys.stderr.write("pdf has zero pages: %s\n" % pdf_path)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
