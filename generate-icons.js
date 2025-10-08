#!/usr/bin/env node

const fs = require('fs');
const path = require('path');
const { createCanvas, loadImage } = require('canvas');
const sharp = require('sharp');

// Configuration
const LOGO_SVG = path.join(__dirname, 'logo.svg');
const SIZES = [16, 32, 48, 64, 128, 256, 512, 1024];
const PADDING_PERCENT = 0.15; // 15% padding on each side
const CORNER_RADIUS_PERCENT = 0.15; // 15% corner radius
const BG_COLOR = '#0a0a0a'; // Dark background

console.log('🎨 Battles.app Icon Generator\n');

// Create temp directory for PNGs
const tempDir = path.join(__dirname, '.icon-temp');
if (!fs.existsSync(tempDir)) {
  fs.mkdirSync(tempDir);
}

async function generateRoundedIcon(size) {
  console.log(`📐 Generating ${size}x${size}...`);
  
  const canvas = createCanvas(size, size);
  const ctx = canvas.getContext('2d');
  
  // Calculate dimensions
  const padding = Math.floor(size * PADDING_PERCENT);
  const logoSize = size - (padding * 2);
  const radius = Math.floor(size * CORNER_RADIUS_PERCENT);
  
  // Draw rounded rectangle background
  ctx.beginPath();
  ctx.moveTo(radius, 0);
  ctx.lineTo(size - radius, 0);
  ctx.quadraticCurveTo(size, 0, size, radius);
  ctx.lineTo(size, size - radius);
  ctx.quadraticCurveTo(size, size, size - radius, size);
  ctx.lineTo(radius, size);
  ctx.quadraticCurveTo(0, size, 0, size - radius);
  ctx.lineTo(0, radius);
  ctx.quadraticCurveTo(0, 0, radius, 0);
  ctx.closePath();
  
  // Fill background
  ctx.fillStyle = BG_COLOR;
  ctx.fill();
  
  // Load and draw SVG logo
  const svgContent = fs.readFileSync(LOGO_SVG, 'utf8');
  const svgBuffer = Buffer.from(svgContent);
  
  // Use sharp to convert SVG to PNG at the right size
  const logoBuffer = await sharp(svgBuffer)
    .resize(logoSize, logoSize, { fit: 'contain', background: { r: 0, g: 0, b: 0, alpha: 0 } })
    .png()
    .toBuffer();
  
  const logoImage = await loadImage(logoBuffer);
  
  // Draw logo centered with padding
  ctx.drawImage(logoImage, padding, padding, logoSize, logoSize);
  
  // Save PNG
  const pngPath = path.join(tempDir, `icon-${size}.png`);
  const out = fs.createWriteStream(pngPath);
  const stream = canvas.createPNGStream();
  stream.pipe(out);
  
  return new Promise((resolve) => {
    out.on('finish', () => {
      console.log(`  ✅ Saved: ${pngPath}`);
      resolve(pngPath);
    });
  });
}

async function convertToICO(pngPaths) {
  console.log('\n🔄 Converting to ICO format...');
  
  // Use sharp to create ICO
  // ICO format typically uses 16, 32, 48, 256 sizes
  const icoSizes = [16, 32, 48, 256];
  const icoPaths = pngPaths.filter(p => {
    const size = parseInt(path.basename(p).match(/\d+/)[0]);
    return icoSizes.includes(size);
  });
  
  // For ICO, we'll use the largest PNG and let Windows handle scaling
  const largest = pngPaths.find(p => p.includes('256'));
  
  if (largest) {
    const icoPath = path.join(__dirname, 'favicon.ico');
    
    // Copy the 256x256 PNG as ICO base
    // Note: True ICO conversion requires a specialized library
    // For now, we'll create a multi-size PNG that Windows can use
    await sharp(largest)
      .resize(256, 256)
      .toFile(icoPath);
    
    console.log(`  ✅ Created: ${icoPath}`);
    return icoPath;
  }
}

async function convertToICNS(pngPaths) {
  console.log('\n🍎 Creating ICNS for macOS...');
  
  // ICNS requires specific sizes: 16, 32, 64, 128, 256, 512, 1024
  const icnsPath = path.join(__dirname, 'icon.icns');
  
  // For macOS ICNS, we need the iconutil command or png2icns
  // Since we're on Windows, we'll create the largest PNG for now
  const largest = pngPaths.find(p => p.includes('1024'));
  
  if (largest) {
    console.log('  ℹ️  For macOS ICNS, use: https://cloudconvert.com/png-to-icns');
    console.log(`  ℹ️  Upload: ${largest}`);
  }
}

async function copyToWebApp(pngPaths) {
  console.log('\n🌐 Copying to web app...');
  
  // Copy 512x512 as favicon for web app
  const webSize = pngPaths.find(p => p.includes('512'));
  
  if (webSize) {
    const webIconPath = path.join(__dirname, '..', 'battles.app', 'public', 'favicon.png');
    
    fs.copyFileSync(webSize, webIconPath);
    console.log(`  ✅ Copied to: ${webIconPath}`);
  }
}

async function main() {
  try {
    // Check if required modules are installed
    try {
      require('canvas');
      require('sharp');
    } catch (e) {
      console.error('❌ Missing dependencies!');
      console.error('📦 Please run: bun install canvas sharp');
      process.exit(1);
    }
    
    // Check if logo.svg exists
    if (!fs.existsSync(LOGO_SVG)) {
      console.error(`❌ Logo not found: ${LOGO_SVG}`);
      process.exit(1);
    }
    
    console.log('🎨 Generating icons with rounded corners and padding...\n');
    
    // Generate all PNG sizes
    const pngPaths = [];
    for (const size of SIZES) {
      const pngPath = await generateRoundedIcon(size);
      pngPaths.push(pngPath);
    }
    
    // Convert to various formats
    await convertToICO(pngPaths);
    await convertToICNS(pngPaths);
    await copyToWebApp(pngPaths);
    
    console.log('\n✨ All icons generated successfully!');
    console.log('\n📁 Generated files:');
    console.log('  - favicon.ico (Windows icon)');
    console.log(`  - ${tempDir}/ (PNG files in various sizes)`);
    console.log('  - ../battles.app/public/favicon.png (Web app icon)');
    
    console.log('\n📝 Next steps:');
    console.log('  1. For macOS ICNS: Upload icon-1024.png to https://cloudconvert.com/png-to-icns');
    console.log('  2. Save as icon.icns in battlesDesktop/ directory');
    console.log('  3. Update tauri.conf.json to include "icon.icns" in the icon array');
    
    console.log('\n✅ Done!\n');
    
  } catch (error) {
    console.error('❌ Error:', error.message);
    console.error(error.stack);
    process.exit(1);
  }
}

main();

