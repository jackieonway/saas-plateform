import base64
import json
import re
from pathlib import Path

import requests


ROOT = Path(__file__).resolve().parent
SKILL = Path(r"C:\Users\MECHREVO\.codex\skills\skills\real-img\SKILL.md")
token = re.search(r"Bearer ([A-Za-z0-9]+)", SKILL.read_text(encoding="utf-8")).group(1)
payload = {
    "model": "gpt-5.6-sol",
    "stream": False,
    "input": [{
        "role": "user",
        "content": (
            "Create an original, high quality 4:5 vertical infographic background for a Chinese personal finance chart. "
            "Warm ivory paper, very subtle cream to white gradient, tasteful muted gold details. "
            "Small elegant three dimensional stack of gold coins with a thin rising arrow at the bottom right, "
            "occupying only the bottom 12 percent. Keep the entire central 85 percent empty, clean, bright, "
            "flat, high contrast, and suitable for overlaid table text. No lettering, no numbers, no symbols, "
            "no grid, no watermark, no borders. Editorial graphic design, restrained, premium."
        ),
    }],
    "tools": [{"type": "image_generation"}],
}
response = requests.post(
    "https://codex.jackieonway.top/v1/responses",
    headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"},
    json=payload,
    timeout=180,
)
response.raise_for_status()
data = response.json()
for item in data.get("output", []):
    if item.get("type") == "image_generation_call" and item.get("result"):
        (ROOT / "savings_background.png").write_bytes(base64.b64decode(item["result"]))
        print("Saved generated background")
        break
else:
    print(json.dumps(data, ensure_ascii=False)[:3000])
    raise RuntimeError("No image_generation_call result")
