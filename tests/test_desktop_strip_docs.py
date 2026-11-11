"""Release and desktop descriptions cover the strip in both languages."""
from pathlib import Path
import unittest
ROOT = Path(__file__).resolve().parents[1]
class DesktopStripDocsTest(unittest.TestCase):
    def test_release_and_readmes_describe_plan_windows_and_vector_concurrency(self):
        for name, words in {
            "README.md": ("fresh plan snapshots", "celestial vector glyphs", "parallel total"),
            "README.zh-TW.md": ("新鮮方案快照", "天體向量圖符", "並行總數"),
            "docs/releases/2.0.1.md": ("Fresh plan snapshots", "Celestial vector glyphs", "新鮮方案快照", "天體向量圖符"),
            "CHANGELOG.md": ("fresh plan snapshots", "celestial vectors", "天體向量圖符"),
        }.items():
            text = (ROOT / name).read_text(encoding="utf-8")
            for word in words:
                with self.subTest(file=name, word=word): self.assertIn(word,text)
