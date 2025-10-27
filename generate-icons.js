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
// Create temp directory for PNGs
const tempDir = path.join(__dirname, '.icon-temp');
if (!fs.existsSync(tempDir)) {
  fs.mkdirSync(tempDir);
}

async function generateTransparentIcon(size) {
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
    return { path: pngPath, buffer: final, size };
    
  } catch (error) {
    throw error;
  }
}

async function convertToICO(icons) {
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
      return icoPath;
    }
  } catch (error) {
  }
}

async function copyToWebApp(icons) {
  try {
    // Copy 512x512 as favicon for web app
    const webIcon = icons.find(icon => icon.size === 512);
    
    if (webIcon) {
      const webAppPublic = path.join(__dirname, '..', 'battles.app', 'public');
      
      // Check if web app directory exists
      if (fs.existsSync(webAppPublic)) {
        const webIconPath = path.join(webAppPublic, 'favicon.png');
        fs.copyFileSync(webIcon.path, webIconPath);
      } else {
      }
    }
  } catch (error) {
  }
}

async function createAppleIcons(icons) {
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
      }
    }
  } catch (error) {
  }
}

async function main() {
  try {
    // Check if required modules are installed
    try {
      require('sharp');
      require('to-ico');
    } catch (e) {
      const { execSync } = require('child_process');
      execSync('bun install', { stdio: 'inherit' });
    }
    
    // Check if logo.svg exists
    if (!fs.existsSync(LOGO_SVG)) {
      process.exit(1);
    }
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
  } catch (error) {
    process.exit(1);
  }
}

main();
