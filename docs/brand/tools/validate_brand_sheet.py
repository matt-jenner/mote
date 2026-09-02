from pathlib import Path
import sys

from pypdf import PdfReader


def validate(pdf: Path) -> None:
    reader = PdfReader(pdf)
    assert len(reader.pages) == 1
    assert reader.metadata.creation_date.isoformat() == "2000-01-01T00:00:00+00:00"
    page = reader.pages[0]
    width = float(page.mediabox.width)
    height = float(page.mediabox.height)
    assert abs(width - 595.276) < 1
    assert abs(height - 841.89) < 1
    text = page.extract_text()
    for expected in [
        "Mote",
        "#45A06B",
        "Fredoka Regular 400",
        "96% width",
        "A simple space for your photos.",
    ]:
        assert expected in text


if __name__ == "__main__":
    filename = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(
        "docs/brand/print/mote-brand-sheet-a4.pdf"
    )
    validate(filename)
    print("brand sheet validated")
