#!/usr/bin/env node

/**
 * Battles.app Desktop - Release Automation Script
 * 
 * This script automates the release process:
 * 1. Builds the Windows executable
 * 2. Generates AI-powered changelog
 * 3. Creates GitHub release with assets
 * 4. Updates version in all necessary files
 */

import { execSync } from 'child_process';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';
import { config } from 'dotenv';
import OpenAI from 'openai';

// Load environment variables from .env file
config();

// Initialize OpenAI
const openai = new OpenAI({
  apiKey: process.env.OPENAI_API_KEY
});

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, '..');

// ANSI color codes
const colors = {
  reset: '\x1b[0m',
  pink: '\x1b[38;2;238;43;99m',    // #ee2b63
  yellow: '\x1b[38;2;233;179;32m', // #e9b320
  white: '\x1b[37m',
  green: '\x1b[32m',
  red: '\x1b[31m',
  cyan: '\x1b[36m'
};

const log = {
  info: (msg) => console.log(`${colors.cyan}ℹ${colors.reset} ${msg}`),
  success: (msg) => console.log(`${colors.green}✓${colors.reset} ${msg}`),
  error: (msg) => console.log(`${colors.red}✗${colors.reset} ${msg}`),
  header: (msg) => console.log(`\n${colors.pink}▶${colors.reset} ${colors.yellow}${msg}${colors.reset}\n`),
  battles: () => console.log(`${colors.pink}█${colors.white}█${colors.yellow}█${colors.reset} Battles.app Desktop Release`)
};

// Get current version from Cargo.toml
function getCurrentVersion() {
  const cargoToml = fs.readFileSync(path.join(rootDir, 'Cargo.toml'), 'utf-8');
  const versionMatch = cargoToml.match(/^version\s*=\s*"([^"]+)"/m);
  return versionMatch ? versionMatch[1] : '0.1.0';
}

// Increment version
function incrementVersion(version, type = 'patch') {
  const [major, minor, patch] = version.split('.').map(Number);
  
  switch (type) {
    case 'major':
      return `${major + 1}.0.0`;
    case 'minor':
      return `${major}.${minor + 1}.0`;
    case 'patch':
    default:
      return `${major}.${minor}.${patch + 1}`;
  }
}

// Update version in Cargo.toml
function updateCargoVersion(newVersion) {
  const cargoPath = path.join(rootDir, 'Cargo.toml');
  let cargoToml = fs.readFileSync(cargoPath, 'utf-8');
  cargoToml = cargoToml.replace(/^version\s*=\s*"[^"]+"/m, `version = "${newVersion}"`);
  fs.writeFileSync(cargoPath, cargoToml);
}

// Update version in tauri.conf.json
function updateTauriVersion(newVersion) {
  const tauriPath = path.join(rootDir, 'tauri.conf.json');
  const tauriConf = JSON.parse(fs.readFileSync(tauriPath, 'utf-8'));
  tauriConf.version = newVersion;
  fs.writeFileSync(tauriPath, JSON.stringify(tauriConf, null, 2) + '\n');
}

// Generate AI-powered changelog from git commits using OpenAI
async function generateChangelog(fromVersion, toVersion) {
  log.header('Generating AI-Powered Changelog');
  
  try {
    // Get git commits since last tag
    const gitLog = execSync(
      `git log v${fromVersion}..HEAD --pretty=format:"%s" --no-merges`,
      { encoding: 'utf-8' }
    ).trim();
    
    if (!gitLog) {
      log.info('No commits found, using default changelog');
      return '### ✨ New Features\n\n• Initial release\n• Stream Deck integration with beautiful branded animations\n• Dark theme with logo colors\n• Real-time FX control for TikTok Live';
    }
    
    const commits = gitLog.split('\n').filter(line => line.trim());
    
    log.info(`Found ${commits.length} commits since v${fromVersion}`);
    log.info('Sending to OpenAI GPT-4 for professional release notes...');
    
    // Use OpenAI to generate professional release notes
    try {
      const response = await openai.chat.completions.create({
        model: 'gpt-4-turbo-preview',
        messages: [
          {
            role: 'system',
            content: `You are a professional release notes writer for Battles.app Desktop, a TikTok Live streaming utility with Elgato Stream Deck integration. 

Transform git commit messages into polished, user-friendly release notes.

Guidelines:
- Group changes into: ✨ New Features, 🚀 Improvements, 🐛 Bug Fixes
- Use clear, non-technical language that users understand
- Highlight user benefits, not implementation details
- Keep each bullet point concise (1-2 lines max)
- Use emojis sparingly (only category headers)
- Focus on what changed for the USER, not the developer
- If Stream Deck is mentioned, emphasize visual/UX improvements
- Mention performance gains if applicable

Example transformation:
"fix streamdeck polling rate" → "Fixed Stream Deck responsiveness with instant button feedback"
"add loading animation" → "Beautiful branded loading animation with smooth gradients and logo colors"

Return ONLY the formatted changelog in markdown, no extra text.`
          },
          {
            role: 'user',
            content: `Generate professional release notes from these commits:\n\n${commits.join('\n')}`
          }
        ],
        temperature: 0.7,
        max_tokens: 1000
      });
      
      const aiChangelog = response.choices[0].message.content.trim();
      log.success('✨ AI-generated changelog created!');
      return aiChangelog;
      
    } catch (aiError) {
      log.warn(`OpenAI API failed: ${aiError.message}`);
      log.info('Falling back to basic changelog generation...');
      
      // Fallback: Basic categorization if AI fails
      const features = [];
      const fixes = [];
      const improvements = [];
      const other = [];
      
      commits.forEach(commit => {
        const lower = commit.toLowerCase();
        if (lower.includes('feat') || lower.includes('add')) {
          features.push(commit.replace(/^(feat|add)[:\s]*/i, ''));
        } else if (lower.includes('fix') || lower.includes('bug')) {
          fixes.push(commit.replace(/^(fix|bug)[:\s]*/i, ''));
        } else if (lower.includes('improve') || lower.includes('optimize') || lower.includes('perf')) {
          improvements.push(commit.replace(/^(improve|optimize|perf)[:\s]*/i, ''));
        } else {
          other.push(commit);
        }
      });
      
      let changelog = '';
      
      if (features.length > 0) {
        changelog += '### ✨ New Features\n\n';
        features.forEach(feat => changelog += `• ${feat}\n`);
        changelog += '\n';
      }
      
      if (improvements.length > 0) {
        changelog += '### 🚀 Improvements\n\n';
        improvements.forEach(imp => changelog += `• ${imp}\n`);
        changelog += '\n';
      }
      
      if (fixes.length > 0) {
        changelog += '### 🐛 Bug Fixes\n\n';
        fixes.forEach(fix => changelog += `• ${fix}\n`);
        changelog += '\n';
      }
      
      if (other.length > 0 && (features.length === 0 && improvements.length === 0 && fixes.length === 0)) {
        changelog += '### 📝 Changes\n\n';
        other.forEach(change => changelog += `• ${change}\n`);
        changelog += '\n';
      }
      
      return changelog || '• Bug fixes and improvements';
    }
  } catch (error) {
    log.error(`Failed to generate changelog: ${error.message}`);
    return '### 🚀 Improvements\n\n• Bug fixes and performance improvements';
  }
}

// Build the application
function buildApp() {
  log.header('Building Application');
  log.info('Building Tauri application for Windows...');
  
  try {
    execSync('bun run tauri build', {
      cwd: rootDir,
      stdio: 'inherit'
    });
    log.success('Build completed successfully!');
    return true;
  } catch (error) {
    log.error('Build failed!');
    return false;
  }
}

// Find the built executable
function findExecutable() {
  const bundleDir = path.join(rootDir, 'src-tauri', 'target', 'release', 'bundle');
  
  log.info('Searching for installer in bundle directory...');
  
  // Look for NSIS installer first (preferred)
  const nsisDir = path.join(bundleDir, 'nsis');
  if (fs.existsSync(nsisDir)) {
    const exeFiles = fs.readdirSync(nsisDir).filter(f => f.endsWith('.exe'));
    if (exeFiles.length > 0) {
      log.success(`Found NSIS installer: ${exeFiles[0]}`);
      return { path: path.join(nsisDir, exeFiles[0]), type: 'exe', name: exeFiles[0] };
    }
  }
  
  // Look for MSI installer as fallback
  const msiDir = path.join(bundleDir, 'msi');
  if (fs.existsSync(msiDir)) {
    const msiFiles = fs.readdirSync(msiDir).filter(f => f.endsWith('.msi'));
    if (msiFiles.length > 0) {
      log.success(`Found MSI installer: ${msiFiles[0]}`);
      return { path: path.join(msiDir, msiFiles[0]), type: 'msi', name: msiFiles[0] };
    }
  }
  
  log.error('No installer found! Expected .exe or .msi in bundle directory');
  return null;
}

// Verify the file is a valid installer (security check)
function verifyInstaller(filePath) {
  const stats = fs.statSync(filePath);
  const filename = path.basename(filePath);
  
  log.info('Verifying installer...');
  log.info(`  File: ${filename}`);
  log.info(`  Size: ${(stats.size / 1024 / 1024).toFixed(2)} MB`);
  
  // Security checks
  if (stats.size < 1024 * 100) { // Less than 100KB is suspicious
    throw new Error('Installer file is too small - possible build failure');
  }
  
  if (stats.size > 1024 * 1024 * 500) { // More than 500MB is suspicious
    throw new Error('Installer file is too large - possible issue');
  }
  
  const ext = path.extname(filePath).toLowerCase();
  if (ext !== '.exe' && ext !== '.msi') {
    throw new Error('Invalid file type - only .exe or .msi allowed');
  }
  
  log.success('Installer verified successfully');
  return true;
}

// Create GitHub release
async function createGitHubRelease(version, changelog, executable) {
  log.header('Creating GitHub Release');
  
  const token = process.env.GITHUB_TOKEN;
  if (!token) {
    log.error('GITHUB_TOKEN not found in .env file!');
    log.info('Please add GITHUB_TOKEN to .env file');
    log.info(`\nManual release instructions:`);
    log.info(`1. Go to: https://github.com/battles-app/desktop-releases/releases/new`);
    log.info(`2. Tag: v${version}`);
    log.info(`3. Title: Battles.app Desktop v${version}`);
    log.info(`4. Description:\n${changelog}`);
    log.info(`5. Upload: ${executable.path}`);
    return false;
  }
  
  // Security: Verify installer before uploading
  try {
    verifyInstaller(executable.path);
  } catch (error) {
    log.error(`Installer verification failed: ${error.message}`);
    log.error('Release aborted for security reasons');
    return false;
  }
  
  try {
    // Create tag
    log.info(`Creating tag v${version}...`);
    execSync(`git tag -a v${version} -m "Release v${version}"`, { cwd: rootDir });
    execSync(`git push origin v${version}`, { cwd: rootDir });
    
    // Prepare release notes (NO source code references)
    log.info('Preparing secure release notes...');
    const releaseNotes = `
# 🎮 Battles.app Desktop v${version}

${changelog}

## 📦 Installation

**Windows 10/11 (64-bit)**

1. Download the installer below
2. Run the setup file
3. Launch Battles.app Desktop
4. Connect your Elgato Stream Deck
5. Login and start streaming!

## ⚠️ Closed Beta

This software is in **closed beta**. Access required:
- Request access in the app
- Or visit: https://battles.app

## 🎨 Features

- 🎭 Real-time animations for TikTok Live
- 💡 Interactive light shows and effects
- 🤖 AI-powered streaming tools
- 🎮 Stream Deck integration
- ⚡ Lightning-fast performance

## 🔗 Links

- 🌐 Website: https://battles.app
- 📧 Support: support@battles.app
- 🐛 Issues: https://github.com/battles-app/desktop-releases/issues

---

**⚠️ Security Notice:** This release contains only the compiled installer. No source code is included.

*Made with ❤️ by the Battles.app team*
`;
    
    // Security: Only upload the installer file, nothing else
    const installerPath = executable.path;
    const installerName = executable.name;
    
    log.info(`Uploading installer: ${installerName}`);
    log.info(`File size: ${(fs.statSync(installerPath).size / 1024 / 1024).toFixed(2)} MB`);
    
    // Create release with ONLY the installer file
    execSync(
      `gh release create v${version} "${installerPath}" --title "Battles.app Desktop v${version}" --notes "${releaseNotes.replace(/"/g, '\\"')}" --repo battles-app/desktop-releases`,
      { cwd: rootDir, stdio: 'inherit' }
    );
    
    log.success(`Release v${version} created successfully!`);
    log.info(`View at: https://github.com/battles-app/desktop-releases/releases/tag/v${version}`);
    return true;
  } catch (error) {
    log.error(`Failed to create GitHub release: ${error.message}`);
    return false;
  }
}

// Main release function
async function release() {
  log.battles();
  console.log('');
  
  // Parse arguments
  const args = process.argv.slice(2);
  const versionType = args[0] || 'patch'; // major, minor, or patch
  
  if (!['major', 'minor', 'patch'].includes(versionType)) {
    log.error(`Invalid version type: ${versionType}`);
    log.info('Usage: bun run release [major|minor|patch]');
    process.exit(1);
  }
  
  // Get current version and calculate new version
  const currentVersion = getCurrentVersion();
  const newVersion = incrementVersion(currentVersion, versionType);
  
  log.header('Release Information');
  log.info(`Current version: ${currentVersion}`);
  log.info(`New version: ${newVersion}`);
  log.info(`Version type: ${versionType}`);
  console.log('');
  
  // Confirm
  if (!process.env.CI) {
    const readline = await import('readline');
    const rl = readline.createInterface({
      input: process.stdin,
      output: process.stdout
    });
    
    await new Promise((resolve) => {
      rl.question(`${colors.yellow}Continue with release v${newVersion}? (y/N): ${colors.reset}`, (answer) => {
        rl.close();
        if (answer.toLowerCase() !== 'y') {
          log.info('Release cancelled');
          process.exit(0);
        }
        resolve();
      });
    });
  }
  
  // Update versions
  log.header('Updating Versions');
  updateCargoVersion(newVersion);
  log.success('Updated Cargo.toml');
  updateTauriVersion(newVersion);
  log.success('Updated tauri.conf.json');
  
  // Generate changelog
  const changelog = await generateChangelog(currentVersion, newVersion);
  log.success('Generated changelog');
  
  // Build
  if (!buildApp()) {
    process.exit(1);
  }
  
  // Find executable
  const executable = findExecutable();
  if (!executable) {
    log.error('Could not find built executable!');
    log.error('Make sure the build completed successfully');
    process.exit(1);
  }
  log.success(`Found installer: ${executable.name}`);
  log.info(`Location: ${executable.path}`);
  
  // Commit version changes
  log.header('Committing Changes');
  try {
    execSync('git add Cargo.toml tauri.conf.json Cargo.lock', { cwd: rootDir });
    execSync(`git commit -m "chore: bump version to ${newVersion}"`, { cwd: rootDir });
    execSync('git push', { cwd: rootDir });
    log.success('Committed and pushed version changes');
  } catch (error) {
    log.error('Failed to commit changes (this is okay if no changes)');
  }
  
  // Create GitHub release
  await createGitHubRelease(newVersion, changelog, executable.path);
  
  log.header('Release Complete! 🎉');
  log.success(`Version ${newVersion} has been released!`);
  console.log('');
}

// Run
release().catch((error) => {
  log.error(`Release failed: ${error.message}`);
  process.exit(1);
});

