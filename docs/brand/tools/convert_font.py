from pathlib import Path
import sys

from fontTools.ttLib import TTFont


def convert(source: Path, destination: Path) -> None:
    font = TTFont(source, recalcTimestamp=False)
    font.flavor = "woff2"
    destination.parent.mkdir(parents=True, exist_ok=True)
    font.save(destination)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("usage: convert_font.py SOURCE.ttf DESTINATION.woff2")
    convert(Path(sys.argv[1]), Path(sys.argv[2]))
