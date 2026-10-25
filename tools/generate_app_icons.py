"""Convert the checked-in original artwork to Windows icon and tile resources.

Requires Pillow. Run from any working directory; no artwork is generated or redrawn.
The original square image is fitted without cropping or changing its background.
"""

from pathlib import Path

from PIL import Image, ImageOps


ASSETS = Path(__file__).resolve().parents[1] / "apps/settings_center/Assets"
SIZES = {
    "LockScreenLogo.scale-200.png": (48, 48),
    "SplashScreen.scale-200.png": (1240, 600),
    "Square150x150Logo.scale-200.png": (300, 300),
    "Square44x44Logo.scale-200.png": (88, 88),
    "Square44x44Logo.targetsize-24_altform-unplated.png": (24, 24),
    "Square44x44Logo.targetsize-48_altform-lightunplated.png": (48, 48),
    "StoreLogo.png": (50, 50),
    "Wide310x150Logo.scale-200.png": (620, 300),
}


def main():
    with Image.open(ASSETS / "AppIconSource.jpeg") as original:
        source = ImageOps.exif_transpose(original).convert("RGBA")
    if source.width != source.height:
        raise ValueError("App icon artwork must be square; do not crop it implicitly")
    source.resize((256, 256), Image.Resampling.LANCZOS).save(
        ASSETS / "AppIcon.ico",
        format="ICO",
        sizes=[(size, size) for size in (16, 20, 24, 32, 40, 48, 64, 96, 128, 256)],
    )
    for name, size in SIZES.items():
        # Non-square tile/splash canvases retain the original dark background.
        canvas = Image.new("RGBA", size, source.getpixel((0, 0)))
        logo = ImageOps.contain(source, size, Image.Resampling.LANCZOS)
        canvas.alpha_composite(logo, ((size[0] - logo.width) // 2, (size[1] - logo.height) // 2))
        canvas.save(ASSETS / name)
    print("Windows app icon and eight tile/splash resources updated from original artwork.")


if __name__ == "__main__":
    main()
