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

// Generate AI changelog from git commits
async function generateChangelog(fromVersion, toVersion) {
  log.header('Generating Changelog');
  
  try {
    // Get git commits since last tag
    const gitLog = execSync(
      `git log v${fromVersion}..HEAD --pretty=format:"%s" --no-merges`,
      { encoding: 'utf-8' }
    ).trim();
    
    if (!gitLog) {
      return '• Initial release\n• Stream Deck integration with beautiful branded animations\n• Dark theme with logo colors\n• Real-time FX control';
    }
    
    const commits = gitLog.split('\n').filter(line => line.trim());
    
    // Categorize commits
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
    
    return changelog;
  } catch (error) {
    log.error(`Failed to generate changelog: ${error.message}`);
    return '• Bug fixes and improvements';
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
  
  // Look for MSI installer
  const msiDir = path.join(bundleDir, 'msi');
  if (fs.existsSync(msiDir)) {
    const msiFiles = fs.readdirSync(msiDir).filter(f => f.endsWith('.msi'));
    if (msiFiles.length > 0) {
      return { path: path.join(msiDir, msiFiles[0]), type: 'msi' };
    }
  }
  
  // Look for NSIS installer
  const nsisDir = path.join(bundleDir, 'nsis');
  if (fs.existsSync(nsisDir)) {
    const exeFiles = fs.readdirSync(nsisDir).filter(f => f.endsWith('.exe'));
    if (exeFiles.length > 0) {
      return { path: path.join(nsisDir, exeFiles[0]), type: 'exe' };
    }
  }
  
  return null;
}

// Create GitHub release
async function createGitHubRelease(version, changelog, exePath) {
  log.header('Creating GitHub Release');
  
  const token = process.env.GITHUB_TOKEN;
  if (!token) {
    log.error('GITHUB_TOKEN environment variable not set!');
    log.info('Please set GITHUB_TOKEN to create releases automatically');
    log.info(`\nManual release instructions:`);
    log.info(`1. Go to: https://github.com/battles-app/desktop-releases/releases/new`);
    log.info(`2. Tag: v${version}`);
    log.info(`3. Title: Battles.app Desktop v${version}`);
    log.info(`4. Description:\n${changelog}`);
    log.info(`5. Upload: ${exePath}`);
    return false;
  }
  
  try {
    // Create tag
    log.info(`Creating tag v${version}...`);
    execSync(`git tag -a v${version} -m "Release v${version}"`, { cwd: rootDir });
    execSync(`git push origin v${version}`, { cwd: rootDir });
    
    // Create release using GitHub CLI
    log.info('Creating GitHub release...');
    const releaseNotes = `
# 🎮 Battles.app Desktop v${version}

${changelog}

## 📦 Installation

**Windows (Stream Deck Required)**

1. Download \`Battles-Desktop-Setup.exe\` below
2. Run the installer
3. Launch Battles.app Desktop
4. Connect your Stream Deck
5. Enjoy the beautiful branded loading animation!

## ⚠️ Closed Beta

This software is in **closed beta**. You need access to use it:
- Request access in the app
- Or visit: https://battles.app

## 🎨 Features

- ✨ Beautiful Stream Deck integration
- 🎬 Branded loading animations with logo colors
- 🎮 Real-time FX control
- 🌊 Smooth dark gradient backgrounds
- ⚡ Lightning-fast performance

## 🔗 Links

- 🌐 Website: https://battles.app
- 📧 Support: support@battles.app
- 🐛 Report Issues: Create an issue in this repository

---

*Made with ${colors.pink}❤${colors.reset} by the Battles.app team*
`;
    
    const fileName = path.basename(exePath);
    execSync(
      `gh release create v${version} "${exePath}" --title "Battles.app Desktop v${version}" --notes "${releaseNotes.replace(/"/g, '\\"')}" --repo battles-app/desktop-releases`,
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
    process.exit(1);
  }
  log.success(`Found executable: ${executable.path}`);
  
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

