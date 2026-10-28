// Raster/ICO compatibility assets are generated from the unchanged SVG masters.
// Requires Node.js + sharp; the runtime UI continues to use the SVG files.
const fs = require('node:fs');
const path = require('node:path');
const sharp = require('sharp');
const assets = path.resolve(__dirname, '../apps/settings_center/Assets');
const brand = path.join(assets, 'Brand');
const sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256];
async function ico(smallSvg, largeSvg, destination) {
  const frames = await Promise.all(sizes.map(size => sharp(size <= 64 ? smallSvg : largeSvg).resize(size, size).png().toBuffer()));
  const header = Buffer.alloc(6 + frames.length * 16);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(frames.length, 4);
  let offset = header.length;
  frames.forEach((frame, index) => {
    const entry = 6 + index * 16;
    header[entry] = sizes[index] % 256;
    header[entry + 1] = sizes[index] % 256;
    header.writeUInt16LE(1, entry + 4);
    header.writeUInt16LE(32, entry + 6);
    header.writeUInt32LE(frame.length, entry + 8);
    header.writeUInt32LE(offset, entry + 12);
    offset += frame.length;
  });
  fs.writeFileSync(destination, Buffer.concat([header, ...frames]));
}
async function main() {
  if (process.argv.includes('--check')) {
    for (const mode of ['Dark', 'Light']) {
      for (const shape of ['Small', 'Rounded', 'Square']) {
        const svg = fs.readFileSync(path.join(brand, `Logo${shape}${mode}.svg`));
        const metadata = await sharp(svg).metadata();
        if (metadata.width !== 255 || metadata.height !== 255) throw Error('Invalid SVG dimensions.');
      }
      const bytes = fs.readFileSync(path.join(brand, `AppIcon.${mode}.ico`));
      if (bytes.readUInt16LE(0) !== 0 || bytes.readUInt16LE(2) !== 1 || bytes.readUInt16LE(4) !== sizes.length)
        throw Error('Invalid ICO header.');
      for (let index = 0; index < sizes.length; ++index) {
        const entry = 6 + index * 16, length = bytes.readUInt32LE(entry + 8), offset = bytes.readUInt32LE(entry + 12);
        if (offset + length > bytes.length) throw Error('ICO frame exceeds file.');
        const frame = bytes.subarray(offset, offset + length);
        const metadata = await sharp(frame).metadata();
        if (metadata.width !== sizes[index] || metadata.height !== sizes[index] || !metadata.hasAlpha)
          throw Error('Missing size or transparency in ICO.');
      }
    }
    if (!fs.readFileSync(path.join(assets, 'AppIcon.ico')).equals(fs.readFileSync(path.join(brand, 'AppIcon.Dark.ico'))))
      throw Error('Static executable icon differs from the SVG fallback.');
    console.log('PASS: six SVG masters and both ICOs, nine sizes each, alpha and executable fallback.');
    return;
  }
  for (const mode of ['Dark', 'Light']) {
    const svg = fs.readFileSync(path.join(brand, `LogoRounded${mode}.svg`));
    const smallSvg = fs.readFileSync(path.join(brand, `LogoSmall${mode}.svg`));
    await ico(smallSvg, svg, path.join(brand, `AppIcon.${mode}.ico`));
    await sharp(svg).resize(255, 255).png().toFile(path.join(brand, `LogoRounded${mode}.png`));
  }
  fs.copyFileSync(path.join(brand, 'AppIcon.Dark.ico'), path.join(assets, 'AppIcon.ico'));
  const dark = fs.readFileSync(path.join(brand, 'LogoRoundedDark.svg'));
  const smallDark = fs.readFileSync(path.join(brand, 'LogoSmallDark.svg'));
  const smallLight = fs.readFileSync(path.join(brand, 'LogoSmallLight.svg'));
  for (const [name, width, height, source] of [
    ['Square44x44Logo.scale-200.png', 88, 88, smallDark],
    ['Square150x150Logo.scale-200.png', 300, 300, dark],
    ['Square44x44Logo.targetsize-24_altform-unplated.png', 24, 24, smallDark],
    ['Square44x44Logo.targetsize-48_altform-lightunplated.png', 48, 48, smallLight],
    ['LockScreenLogo.scale-200.png', 48, 48, smallDark],
    ['StoreLogo.png', 50, 50, smallDark],
    ['Wide310x150Logo.scale-200.png', 620, 300, dark],
    ['SplashScreen.scale-200.png', 1240, 600, dark],
  ]) await sharp(source).resize(width, height, {fit: 'contain', background: '#00000000'}).png().toFile(path.join(assets, name));
  console.log('Generated both themed ICOs (9 sizes each), PNG fallbacks and Windows logo assets.');
}
main().catch(error => { console.error(error); process.exitCode = 1; });
