#!/usr/bin/env node

const fs = require('fs');
const path = require('path');
const sharp = require('sharp');
const toIco = require('to-ico');

// Configuration
const LOGO_SVG = path.join(__dirname, 'logo.svg');
const SIZES = [16, 32, 48, 64, 128, 256, 512, 1024];
const PADDING_PERCENT = 0.12; // 12% padding on each side (slightly less for better visibility)
const CORNER_RADIUS_PERCENT = 0.0; // No corner radius - full transparency

console.log('🎨 Battles.app Icon Generator\n');

// Create temp directory for PNGs
const tempDir = path.join(__dirname, '.icon-temp');
if (!fs.existsSync(tempDir)) {
  fs.mkdirSync(tempDir);
}

async function generateTransparentIcon(size) {
  console.log(`📐 Generating ${size}x${size}...`);
  
  // Calculate dimensions
  const padding = Math.floor(size * PADDING_PERCENT);
  const logoSize = size - (padding * 2);
  
  try {
    // Create transparent canvas
    const transparentBg = await sharp({
      create: {
        width: size,
        height: size,
        channels: 4,
        background: { r: 0, g: 0, b: 0, alpha: 0 } // Fully transparent
      }
    })
    .png()
    .toBuffer();
    
    // Load and resize SVG logo with high quality
    const logoBuffer = await sharp(LOGO_SVG)
      .resize(logoSize, logoSize, { 
        fit: 'contain',
        kernel: 'lanczos3', // High-quality downsampling
        background: { r: 0, g: 0, b: 0, alpha: 0 }
      })
      .png()
      .toBuffer();
    
    // Composite logo on transparent background with padding
    const final = await sharp(transparentBg)
      .composite([
        {
          input: logoBuffer,
          left: padding,
          top: padding,
          blend: 'over'
        }
      ])
      .png()
      .toBuffer();
    
    // Save PNG
    const pngPath = path.join(tempDir, `icon-${size}.png`);
    await sharp(final).toFile(pngPath);
    
    console.log(`  ✅ Saved: icon-${size}.png`);
    return { path: pngPath, buffer: final, size };
    
  } catch (error) {
    console.error(`  ❌ Error generating ${size}x${size}:`, error.message);
    throw error;
  }
}

async function convertToICO(icons) {
  console.log('\n🔄 Converting to ICO format...');
  
  try {
    // ICO format supports 16, 32, 48, 256 sizes
    const icoSizes = [16, 32, 48, 256];
    const icoBuffers = icons
      .filter(icon => icoSizes.includes(icon.size))
      .map(icon => icon.buffer);
    
    if (icoBuffers.length > 0) {
      const icoBuffer = await toIco(icoBuffers);
      const icoPath = path.join(__dirname, 'favicon.ico');
      
      fs.writeFileSync(icoPath, icoBuffer);
      console.log(`  ✅ Created: favicon.ico (${icoBuffers.length} sizes)`);
      return icoPath;
    }
  } catch (error) {
    console.error('  ❌ Error creating ICO:', error.message);
  }
}

async function copyToWebApp(icons) {
  console.log('\n🌐 Copying to web app...');
  
  try {
    // Copy 512x512 as favicon for web app
    const webIcon = icons.find(icon => icon.size === 512);
    
    if (webIcon) {
      const webAppPublic = path.join(__dirname, '..', 'battles.app', 'public');
      
      // Check if web app directory exists
      if (fs.existsSync(webAppPublic)) {
        const webIconPath = path.join(webAppPublic, 'favicon.png');
        fs.copyFileSync(webIcon.path, webIconPath);
        console.log(`  ✅ Copied to: battles.app/public/favicon.png`);
      } else {
        console.log(`  ⚠️  Web app directory not found: ${webAppPublic}`);
      }
    }
  } catch (error) {
    console.error('  ❌ Error copying to web app:', error.message);
  }
}

async function createAppleIcons(icons) {
  console.log('\n🍎 Creating Apple Touch Icons...');
  
  try {
    const appleIcon = icons.find(icon => icon.size === 180);
    if (!appleIcon) {
      // Generate 180x180 specifically for Apple
      const apple = await generateTransparentIcon(180);
      icons.push(apple);
    }
    
    const webAppPublic = path.join(__dirname, '..', 'battles.app', 'public');
    if (fs.existsSync(webAppPublic)) {
      const appleIconPath = path.join(webAppPublic, 'apple-touch-icon.png');
      const icon180 = icons.find(icon => icon.size === 180);
      if (icon180) {
        fs.copyFileSync(icon180.path, appleIconPath);
        console.log(`  ✅ Created: apple-touch-icon.png (180x180)`);
      }
    }
  } catch (error) {
    console.error('  ❌ Error creating Apple icons:', error.message);
  }
}

async function main() {
  try {
    // Check if required modules are installed
    try {
      require('sharp');
      require('to-ico');
    } catch (e) {
      console.error('❌ Missing dependencies!');
      console.error('📦 Installing dependencies...\n');
      const { execSync } = require('child_process');
      execSync('bun install', { stdio: 'inherit' });
    }
    
    // Check if logo.svg exists
    if (!fs.existsSync(LOGO_SVG)) {
      console.error(`❌ Logo not found: ${LOGO_SVG}`);
      process.exit(1);
    }
    
    console.log('🎨 Generating transparent high-quality icons with padding...\n');
    
    // Generate all PNG sizes
    const icons = [];
    for (const size of SIZES) {
      const icon = await generateTransparentIcon(size);
      icons.push(icon);
    }
    
    // Convert to various formats
    await convertToICO(icons);
    await copyToWebApp(icons);
    await createAppleIcons(icons);
    
    console.log('\n✨ All icons generated successfully!');
    console.log('\n📁 Generated files:');
    console.log('  ✅ favicon.ico (Windows icon - 16, 32, 48, 256)');
    console.log(`  ✅ .icon-temp/ (PNG files: ${SIZES.join(', ')} px)`);
    console.log('  ✅ battles.app/public/favicon.png (512x512)');
    console.log('  ✅ battles.app/public/apple-touch-icon.png (180x180)');
    
    console.log('\n📝 Icon features:');
    console.log(`  • ${PADDING_PERCENT * 100}% padding on all sides`);
    console.log('  • Fully transparent background');
    console.log('  • High-quality lanczos3 scaling');
    console.log('  • No background or corners');
    
    console.log('\n📝 Optional - macOS ICNS:');
    console.log('  1. Upload .icon-temp/icon-1024.png to: https://cloudconvert.com/png-to-icns');
    console.log('  2. Save as icon.icns in battlesDesktop/ directory');
    console.log('  3. Update tauri.conf.json to include "icon.icns" in the icon array');
    
    console.log('\n✅ Done!\n');
    
  } catch (error) {
    console.error('\n❌ Fatal error:', error.message);
    console.error(error.stack);
    process.exit(1);
  }
}

main();
