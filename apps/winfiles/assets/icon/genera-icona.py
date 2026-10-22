"""Disegna l'icona di WinFiles (quadrato blu arrotondato con la W in Inter SemiBold), come il
marchio in alto a sinistra dell'app. Scrive icona.ico (Esplora file, menu Start, installer) e
icona-256.png (icona della finestra). Uso: python3 genera-icona.py (serve Pillow)."""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

QUI = Path(__file__).resolve().parent
FONT = QUI.parent / "fonts" / "Inter-SemiBold.ttf"
BLU = (37, 99, 235, 255)  # accent del tema (#2563eb)


def disegna(lato: int) -> Image.Image:
    # Disegnata a 4 volte e ridotta: bordi morbidi anche a 16 pixel.
    s = lato * 4
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    margine = round(s * 0.04)
    d.rounded_rectangle((margine, margine, s - margine, s - margine), radius=round(s * 0.24), fill=BLU)
    font = ImageFont.truetype(str(FONT), round(s * 0.62))
    d.text((s / 2, s / 2 + s * 0.02), "W", font=font, fill=(255, 255, 255, 255), anchor="mm")
    return img.resize((lato, lato), Image.LANCZOS)


grande = disegna(256)
grande.save(QUI / "icona-256.png")
# Immagini BMP, non PNG: con le icone piccole in PNG la barra delle applicazioni e i collegamenti
# di Windows mostravano un rettangolo bianco.
grande.save(
    QUI / "icona.ico",
    sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    bitmap_format="bmp",
)
