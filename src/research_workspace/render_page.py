import hashlib
import json
import sys

try:
    import pypdfium2 as pdfium
except ImportError as exc:
    print(str(exc), file=sys.stderr)
    sys.exit(3)

pdf_path, expected, page_arg, scale_arg, out = sys.argv[1:]
try:
    with open(pdf_path, 'rb') as f:
        hasher = hashlib.sha256()
        for block in iter(lambda: f.read(1024 * 1024), b''):
            hasher.update(block)
        digest = hasher.hexdigest()
except OSError as exc:
    print(str(exc), file=sys.stderr)
    sys.exit(4)
if digest.lower() != expected.lower():
    sys.exit(5)
try:
    pdf = pdfium.PdfDocument(pdf_path)
    page_number = int(page_arg)
    if page_number < 1 or page_number > len(pdf):
        sys.exit(6)
    page = pdf[page_number - 1]
    width, height = page.get_size()
    rotation = page.get_rotation()
    image = page.render(scale=float(scale_arg)).to_pil()
except Exception as exc:
    print(str(exc), file=sys.stderr)
    sys.exit(4)
image.save(out, format='PNG')
print(json.dumps(dict(path=out, width_px=image.width, height_px=image.height,
                      page_width_pt=width, page_height_pt=height, rotation=rotation)))
