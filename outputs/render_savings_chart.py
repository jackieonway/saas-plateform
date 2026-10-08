from math import ceil
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


ROOT = Path(__file__).resolve().parent
WIDTH, HEIGHT = 2000, 2400
BG = Image.open(ROOT / "savings_background.png").convert("RGB")
BG = BG.resize((WIDTH, HEIGHT), Image.Resampling.LANCZOS)
canvas = BG.convert("RGBA")
veil = Image.new("RGBA", (WIDTH, HEIGHT), (255, 255, 255, 0))
d = ImageDraw.Draw(veil)
d.rounded_rectangle((64, 60, 1936, 2320), radius=58, fill=(255, 253, 248, 246), outline=(219, 191, 145, 140), width=3)
canvas = Image.alpha_composite(canvas, veil)
d = ImageDraw.Draw(canvas)

font = r"C:\Windows\Fonts\msyh.ttc"
bold = r"C:\Windows\Fonts\msyhbd.ttc"
def f(size, weight=False):
    return ImageFont.truetype(bold if weight else font, size)

ink = (47, 34, 28)
muted = (112, 98, 83)
gold = (167, 114, 50)
line = (223, 211, 193)

def centered(text, center_x, top, face, color):
    box = d.textbbox((0, 0), text, font=face)
    d.text((center_x - (box[2] - box[0]) / 2, top), text, font=face, fill=color)


d.text((132, 117), "存到目标金额，要多久？", font=f(84, True), fill=ink)
d.text((136, 246), "按参考图的工资档位，增加 1 万和 2 万月收入测算", font=f(38), fill=muted)
d.rounded_rectangle((128, 326, 1872, 420), radius=24, fill=(247, 236, 218))
d.text((164, 344), "税后月收入  ·  每月存 50%  ·  从零开始", font=f(42, True), fill=(108, 69, 29))

cols = [300, 670, 965, 1260, 1555, 1765]
headers = ["月收入", "100万", "200万", "300万", "400万", "500万"]
d.rounded_rectangle((115, 476, 1885, 570), radius=19, fill=(133, 87, 42))
for i, head in enumerate(headers):
    centered(head, cols[i], 489, f(43, True), (255, 255, 255))

salaries = [3000, 5000, 8000, 10000, 15000, 20000, 25000, 40000, 60000, 80000, 240000, 400000]
targets = [1_000_000, 2_000_000, 3_000_000, 4_000_000, 5_000_000]
row_top = 584
row_h = 125
for idx, salary in enumerate(salaries):
    y = row_top + idx * row_h
    if idx % 2 == 0:
        d.rounded_rectangle((115, y, 1885, y + row_h - 5), radius=12, fill=(250, 245, 236))
    centered(f"¥{salary:,}", cols[0], y + 20, f(40, True), ink)
    for j, target in enumerate(targets, 1):
        months = ceil(target / (salary * 0.5))
        value = f"{months}个月" if months < 12 else f"{months / 12:.1f}年"
        centered(value, cols[j], y + 21, f(39, j == 1), gold if j == 1 else ink)
    d.line((132, y + row_h - 2, 1868, y + row_h - 2), fill=line, width=2)

d.line((128, 2130, 1872, 2130), fill=(201, 175, 137), width=3)
d.text((134, 2160), "说明：以整月攒够为准；不计利息、加薪、通胀。", font=f(38), fill=muted)
d.text((134, 2220), "实际所需时间取决于收入和储蓄比例。", font=f(38), fill=muted)

canvas.convert("RGB").save(ROOT / "存款目标时间对照图.png", quality=95)
print(ROOT / "存款目标时间对照图.png")
