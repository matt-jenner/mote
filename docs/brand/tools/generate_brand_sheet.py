from pathlib import Path
import sys

from reportlab.lib.colors import HexColor
from reportlab.lib.pagesizes import A4
from reportlab.lib.utils import ImageReader
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.pdfgen import canvas


GREEN = HexColor("#45A06B")
GRAPHITE = HexColor("#171A1F")
WHITE = HexColor("#F7F8FA")
GREY = HexColor("#B9C1C9")


def draw_image_fit(page, filename: Path, x: float, y: float, width: float, height: float) -> None:
    image = ImageReader(filename)
    source_width, source_height = image.getSize()
    scale = min(width / source_width, height / source_height)
    target_width = source_width * scale
    target_height = source_height * scale
    page.drawImage(
        image,
        x + (width - target_width) / 2,
        y + (height - target_height) / 2,
        target_width,
        target_height,
        mask="auto",
    )


def build(light_lockup: Path, dark_lockup: Path, output: Path) -> None:
    brand_root = output.parent.parent
    font_path = brand_root / "fonts/Fredoka-Variable.ttf"
    icon_path = brand_root / "icons/web/icon-512.png"
    pdfmetrics.registerFont(TTFont("Fredoka", font_path))

    output.parent.mkdir(parents=True, exist_ok=True)
    width, height = A4
    page = canvas.Canvas(str(output), pagesize=A4, pageCompression=1, invariant=1)
    page.setTitle("Mote brand sheet")
    page.setAuthor("Mote")
    page.setSubject("Mote logo, palette, typography, and usage guidance")

    page.setFillColor(WHITE)
    page.rect(0, 0, width, height, fill=1, stroke=0)

    margin = 42
    page.setFillColor(GREEN)
    page.roundRect(margin, height - 54, 28, 4, 2, fill=1, stroke=0)
    page.setFillColor(GRAPHITE)
    page.setFont("Helvetica-Bold", 8)
    page.drawString(margin + 38, height - 56, "MOTE BRAND ASSETS  /  2026")

    draw_image_fit(page, light_lockup, margin, height - 216, width - 2 * margin, 122)
    page.setFont("Fredoka", 16)
    page.drawString(margin, height - 238, "A simple space for your photos.")

    dark_y = height - 424
    page.setFillColor(GRAPHITE)
    page.roundRect(margin, dark_y, width - 2 * margin, 150, 16, fill=1, stroke=0)
    draw_image_fit(page, dark_lockup, margin + 28, dark_y + 25, width - 2 * margin - 56, 100)

    section_y = dark_y - 42
    page.setFillColor(GRAPHITE)
    page.setFont("Helvetica-Bold", 9)
    page.drawString(margin, section_y, "COLOUR")

    swatches = [
        ("#45A06B", GREEN),
        ("#171A1F", GRAPHITE),
        ("#F7F8FA", WHITE),
        ("#B9C1C9", GREY),
    ]
    swatch_y = section_y - 62
    swatch_width = 92
    gap = 18
    for index, (label, colour) in enumerate(swatches):
        x = margin + index * (swatch_width + gap)
        page.setFillColor(colour)
        page.setStrokeColor(GREY)
        page.roundRect(x, swatch_y + 18, swatch_width, 32, 8, fill=1, stroke=1)
        page.setFillColor(GRAPHITE)
        page.setFont("Helvetica", 8)
        page.drawString(x, swatch_y, label)

    icon_x = width - margin - 74
    draw_image_fit(page, icon_path, icon_x, swatch_y - 2, 74, 74)

    rule_y = swatch_y - 30
    page.setStrokeColor(GREY)
    page.setLineWidth(0.6)
    page.line(margin, rule_y, width - margin, rule_y)

    page.setFillColor(GRAPHITE)
    page.setFont("Helvetica-Bold", 9)
    page.drawString(margin, rule_y - 30, "TYPE")
    page.setFont("Fredoka", 22)
    page.drawString(margin, rule_y - 62, "Mote")
    page.setFont("Helvetica", 9)
    page.drawString(margin + 105, rule_y - 54, "Fredoka Regular 400")
    page.drawString(margin + 105, rule_y - 69, "96% width  /  -3.5% tracking")

    page.setFont("Helvetica-Bold", 9)
    page.drawString(330, rule_y - 30, "USE")
    page.setFont("Helvetica", 8.5)
    page.drawString(330, rule_y - 49, "Clear space: one centre-square width")
    page.drawString(330, rule_y - 64, "Minimum symbol: 16 px screen / 6 mm print")
    page.drawString(330, rule_y - 79, "Keep the wordmark outside app icons")

    footer_y = 34
    page.setStrokeColor(GREY)
    page.line(margin, footer_y + 18, width - margin, footer_y + 18)
    page.setFillColor(GRAPHITE)
    page.setFont("Helvetica", 7.5)
    page.drawString(margin, footer_y, "Fredoka is distributed under the SIL Open Font License 1.1.")
    page.drawRightString(width - margin, footer_y, "Mote  /  Brand sheet")

    page.showPage()
    page.save()


if __name__ == "__main__":
    if len(sys.argv) != 4:
        raise SystemExit(
            "usage: generate_brand_sheet.py LIGHT_LOCKUP.png DARK_LOCKUP.png OUTPUT.pdf"
        )
    build(Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3]))
