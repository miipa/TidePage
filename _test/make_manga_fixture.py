# -*- coding: utf-8 -*-
"""生成「漫画模式纵向瀑布流」可复现夹具：一本 N 章、每章 1 张大图的 EPUB。

- 每章 1 个 spine item、1 张图 → is_manga_like(images>=30 && images>=spine*0.8) 判定为漫画。
- 每张图 200~600KB 且高 > 视口高度（1000px），使 1 章 ≈ 1 屏，便于断言滚动边界。
- 图为手写 PNG（zlib level 6）：顶部 noise 带（不可压缩，精确控制字节数）+ 底部平滑渐变。

用法：python _test/make_manga_fixture.py <输出路径> [章数]
"""
import os
import struct
import sys
import zlib

W, H = 900, 1100
TARGET = 380_000  # 目标字节数（落在 200~600KB 区间中部）


def png_bytes(seed, target=TARGET):
    """构造 W×H 真彩 PNG：顶部 noise 带负责字节数，底部渐变保证内容「像一页图」。"""
    rnd = _Lcg(seed)
    band = max(1, min(H - 40, int(target / (W * 3)) + 2))
    raw = bytearray()
    for y in range(H):
        raw.append(0)  # filter type 0
        if y < band:
            # 不可压缩噪声：像素值来自确定性伪随机序列
            row = bytes(rnd() & 0xFF for _ in range(W * 3))
            # 叠一层可视纹理（页码/条纹），避免整图纯噪点导致解码器走慢路径
            if (y // 40) % 2 == 0:
                row = bytes(min(255, b + 60) for b in row)
            raw += row
        else:
            base = (y * 255) // H
            row = bytearray()
            for x in range(W):
                v = (base + (x * 255) // W) // 2 + 96
                row += bytes((v & 0xFF, (v * 2) & 0xFF, 255 - (v & 0xFF)))
            raw += row
    idat = zlib.compress(bytes(raw), 6)
    out = bytearray(b"\x89PNG\r\n\x1a\n")

    def chunk(tag, data):
        out.extend(struct.pack(">I", len(data)))
        out.extend(tag)
        out.extend(data)
        out.extend(struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF))

    chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0))
    chunk(b"IDAT", idat)
    chunk(b"IEND", b"")
    return bytes(out)


class _Lcg:
    def __init__(self, seed):
        self.s = seed & 0xFFFFFFFF

    def __call__(self):
        self.s = (1103515245 * self.s + 12345) & 0x7FFFFFFF
        return self.s >> 7


def build(out_path, chapters=60):
    imgs = [png_bytes(1000 + i) for i in range(chapters)]
    for i, b in enumerate(imgs):
        assert 200_000 <= len(b) <= 600_000, (i, len(b))
    spine = "\n".join(
        f'    <itemref idref="c{i}" linear="yes"/>' for i in range(chapters)
    )
    manifest = "\n".join(
        f'    <item id="p{i}" href="c{i:03d}.xhtml" media-type="application/xhtml+xml"/>'
        for i in range(chapters)
    ) + "\n" + "\n".join(
        f'    <item id="i{i}" href="img/{i:03d}.png" media-type="image/png"/>'
        for i in range(chapters)
    ) + '\n    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>'
    nav_links = "\n".join(
        f'        <li><a href="c{i:03d}.xhtml">第{i + 1}话</a></li>' for i in range(chapters)
    )
    opf = f"""<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="bid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="bid">manga-flow-fixture</dc:identifier>
    <dc:title>漫画瀑布流夹具</dc:title>
    <dc:language>zh-CN</dc:language>
  </metadata>
  <manifest>
{manifest}
  </manifest>
  <spine>
{spine}
  </spine>
</package>
"""
    nav = f"""<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>目录</title></head>
<body>
  <nav epub:type="toc" id="toc">
    <h1>目录</h1>
    <ol>
{nav_links}
    </ol>
  </nav>
</body>
</html>
"""
    container = """<?xml version="1.0" encoding="utf-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"""
    import zipfile

    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    with zipfile.ZipFile(out_path, "w", zipfile.ZIP_DEFLATED) as z:
        # mimetype 必须是首个且 stored（EPUB 规范）
        z.writestr(zipfile.ZipInfo("mimetype"), "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        z.writestr("META-INF/container.xml", container)
        z.writestr("OEBPS/content.opf", opf)
        z.writestr("OEBPS/nav.xhtml", nav)
        for i in range(chapters):
            page = f"""<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>第{i + 1}话</title></head>
<body><div><img src="img/{i:03d}.png" alt="p{i + 1}"/></div></body>
</html>
"""
            z.writestr(f"OEBPS/c{i:03d}.xhtml", page)
        for i, b in enumerate(imgs):
            z.writestr(f"OEBPS/img/{i:03d}.png", b)
    print(f"wrote {out_path} chapters={chapters} bytes={os.path.getsize(out_path)} img0={len(imgs[0])}B")


if __name__ == "__main__":
    out = sys.argv[1]
    n = int(sys.argv[2]) if len(sys.argv) > 2 else 60
    build(out, n)