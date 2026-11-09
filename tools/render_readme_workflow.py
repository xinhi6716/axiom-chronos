#!/usr/bin/env python3
"""Generate localized README workflow SVGs using only the Python standard library."""
from __future__ import annotations

import argparse
from html import escape
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PRODUCT = 'chronos'
COPY = {'en': {'title': 'From a wish to verified work', 'desc': 'Astra plans, the critic reviews the plan, the architect splits modules, Codex builds, Claude reviews the exact files, then full tests, CI and memory close the loop.', 'steps': [['Astra', 'Define the change', 'Plan first'], ['Critic', 'Review the plan', 'Core rules before build'], ['Architect', 'Split into modules', 'Contracts + tests'], ['Codex', 'terra builds each unit', 'sol integrates if needed'], ['Claude review', 'Review exact files', 'Cross-family check'], ['Verify', 'Full tests + CI', 'Record what changed']], 'note': 'Review each diff before committing.', 'mobile_title': ['From a wish', 'to verified work']}, 'zh-TW': {'title': '從願望到驗證完成', 'desc': 'Astra 先規劃、critic 審查計畫、architect 切模組、Codex 施工、Claude 審查確切檔案，最後由完整測試、CI 與記憶收尾。', 'steps': [['Astra', '定義修改目標', '先完成計畫'], ['Critic', '審查計畫', '核心裁決後才施工'], ['Architect', '切分工作模組', '介面規格與驗收測試'], ['Codex', 'terra 實作各單元', '必要時由 sol 整合'], ['Claude 審查', '檢查確切檔案', '跨模型家族複核'], ['驗證', '完整測試與 CI', '保存工作紀錄']], 'note': '每次 commit 前請檢查 diff。', 'mobile_title': ['從願望', '到驗證完成']}}
PALETTES = {
    "light": dict(bg="#f4efe3", panel="#fbf8f0", ink="#101a22", muted="#5b5548",
                  gold="#7e5c22", line="#cabc9e", ornament="#8d6b2d"),
    "dark": dict(bg="#101a22", panel="#172530", ink="#f4efe3", muted="#c9c4b6",
                 gold="#d7bd82", line="#586052", ornament="#b89a58"),
}


def text(x, y, value, *, size=24, color="ink", family="sans", anchor="start"):
    families = {
        "sans": "Arial, 'Noto Sans CJK TC', 'Microsoft JhengHei', sans-serif",
        "serif": "Georgia, 'Noto Serif CJK TC', 'Times New Roman', serif",
        "mono": "'Courier New', monospace",
    }
    return (f'<text x="{x}" y="{y}" font-size="{size}" fill="{{{color}}}" '
            f'font-family="{families[family]}" text-anchor="{anchor}">{escape(value)}</text>')


def filename(language, theme, mobile):
    suffix = "" if language == "en" else "-" + language
    return (PRODUCT + "-process" + suffix + ("-dark" if theme == "dark" else "")
            + ("-mobile" if mobile else "") + ".svg")


def render(language, theme, mobile):
    copy = COPY[language]
    count = len(copy["steps"])
    width, height = (360, 200 + 132 * count) if mobile else (1200, 594)
    out = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
           f'viewBox="0 0 {width} {height}" role="img" aria-labelledby="title desc" '
           f'xml:lang="{language}">',
           f'<title id="title">Axiom {PRODUCT.title()}: {escape(copy["title"])}</title>',
           f'<desc id="desc">{escape(copy["desc"])}</desc>',
           f'<rect width="{width}" height="{height}" fill="{{bg}}"/>',
           f'<path d="M24 1H{width-24}M24 {height-1}H{width-24}" stroke="{{line}}"/>']
    if mobile:
        out += [text(20, 32, 'AXIOM / ' + PRODUCT.upper(), size=15, color="gold", family="mono"),
                text(20, 70, copy["mobile_title"][0], size=28, family="serif"),
                text(20, 103, copy["mobile_title"][1], size=28, family="serif")]
        for i, (title, first, second) in enumerate(copy["steps"]):
            y = 126 + 132 * i
            out.append(f'<g data-step="{i+1}">')
            out += [f'<rect x="20" y="{y}" width="320" height="116" rx="6" fill="{{panel}}" stroke="{{line}}"/>',
                    f'<circle cx="47" cy="{y+31}" r="17" fill="{{bg}}" stroke="{{ornament}}"/>',
                    text(47, y+37, f'{i+1:02}', size=16, color="gold", family="mono", anchor="middle"),
                    text(76, y+38, title, size=25, family="serif"),
                    text(36, y+72, first, size=20, color="muted"),
                    text(36, y+99, second, size=20, color="muted"), '</g>']
            if i < count-1:
                out.append(f'<path d="M47 {y+116}v12m-4-4 4 4 4-4" fill="none" stroke="{{ornament}}" stroke-width="1.5"/>')
        if PRODUCT == 'chronos':
            notes = ['Review each diff', 'before committing.'] if language == 'en' else ['每次 commit 前', '請檢查 diff。']
        else:
            notes = {'en': ['For your existing agent.', 'Not a separate engine.'],
                     'zh-TW': ['給現有 agent 的技能包，', '不是另一套執行引擎。'],
                     'zh-CN': ['给现有 agent 的技能包，', '不是另一套执行引擎。']}[language]
        out += [text(20, height-41, notes[0], size=18, color="gold"),
                text(20, height-16, notes[1], size=18, color="gold")]
    else:
        columns = 3 if count == 6 else 2
        card_width = (1128 - 24*(columns-1)) // columns
        out += [text(36, 39, 'AXIOM / ' + PRODUCT.upper(), size=19, color="gold", family="mono"),
                text(36, 99, copy["title"], size=43, family="serif"),
                '<path d="M36 124H1164" stroke="{line}"/>']
        for i, (title, first, second) in enumerate(copy["steps"]):
            x = 36 + (card_width+24) * (i % columns)
            y = 150 + 184 * (i // columns)
            out.append(f'<g data-step="{i+1}">')
            out += [f'<rect x="{x}" y="{y}" width="{card_width}" height="160" rx="6" fill="{{panel}}" stroke="{{line}}"/>',
                    f'<circle cx="{x+38}" cy="{y+39}" r="20" fill="{{bg}}" stroke="{{ornament}}"/>',
                    text(x+38, y+46, f'{i+1:02}', size=19, color="gold", family="mono", anchor="middle"),
                    text(x+72, y+49, title, size=31, family="serif"),
                    text(x+24, y+96, first, color="muted"),
                    text(x+24, y+130, second, color="muted"), '</g>']
            if i % columns < columns-1:
                out.append(f'<path d="M{x+card_width+4} {y+80}h16m-5-5 5 5-5 5" '
                           'fill="none" stroke="{ornament}" stroke-width="1.5"/>')
        # Numbers keep the reading order explicit. Rows always read left to right.
        out += ['<path d="M36 526H1164" stroke="{line}"/>',
                text(36, 563, copy["note"], size=23, color="gold")]
    out.append('</svg>')
    result = '\n'.join(out) + '\n'
    for name, value in PALETTES[theme].items():
        result = result.replace('{' + name + '}', value)
    return result


def assets():
    for language in COPY:
        for theme in PALETTES:
            for mobile in (False, True):
                yield ROOT / 'assets' / filename(language, theme, mobile), render(language, theme, mobile)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='Fail if checked-in SVGs differ from their source.')
    args = parser.parse_args()
    stale = []
    for path, content in assets():
        if args.check:
            if not path.exists() or path.read_text(encoding='utf-8') != content:
                stale.append(str(path.relative_to(ROOT)))
        else:
            path.write_text(content, encoding='utf-8')
    if stale:
        parser.error('Regenerate workflow assets: ' + ', '.join(stale))
    print(f'{len(list(assets()))} workflow assets {"checked" if args.check else "generated"}.')


if __name__ == '__main__':
    main()
