#!/usr/bin/env node

/**
 * Create battles-app/desktop GitHub Repository
 * 
 * Sets up the repository with proper description, topics, and settings
 */

import { config } from 'dotenv';
import { execSync } from 'child_process';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, '..');

// Load environment variables
config();

const REPO_CONFIG = {
  name: 'desktop',
  org: 'battles-app',
  description: '🎮 Battles.app Desktop - Pro TikTok Live Utilities with Elgato Stream Deck Integration (Windows)',
  homepage: 'https://battles.app',
  topics: [
    'tiktok',
    'streaming',
    'elgato-streamdeck',
    'desktop-app',
    'live-streaming',
    'windows',
    'tauri',
    'rust'
  ],
  private: false,
  has_issues: true,
  has_projects: false,
  has_wiki: false,
  has_downloads: true
};

const colors = {
  reset: '\x1b[0m',
  cyan: '\x1b[36m',
  green: '\x1b[32m',
  red: '\x1b[31m',
  yellow: '\x1b[33m'
};

function log(message, color = 'reset') {
  console.log(`${colors[color]}${message}${colors.reset}`);
}

async function createRepository() {
  log('\n════════════════════════════════════════', 'cyan');
  log('  Creating battles-app/desktop Repository', 'cyan');
  log('════════════════════════════════════════\n', 'cyan');

  try {
    // Check if repository already exists
    log('Checking if repository exists...', 'cyan');
    try {
      execSync(`gh repo view ${REPO_CONFIG.org}/${REPO_CONFIG.name}`, { 
        stdio: 'ignore' 
      });
      log(`✅ Repository ${REPO_CONFIG.org}/${REPO_CONFIG.name} already exists!`, 'green');
      log('Updating repository settings...', 'cyan');
    } catch (error) {
      // Repository doesn't exist, create it
      log('Creating new repository...', 'cyan');
      
      const createCmd = `gh repo create ${REPO_CONFIG.org}/${REPO_CONFIG.name} --public --description "${REPO_CONFIG.description}" --homepage "${REPO_CONFIG.homepage}"`;
      
      execSync(createCmd, { stdio: 'inherit' });
      log(`✅ Repository created: ${REPO_CONFIG.org}/${REPO_CONFIG.name}`, 'green');
    }

    // Update repository settings
    log('\nConfiguring repository...', 'cyan');
    
    // Set topics
    const topicsJson = JSON.stringify(REPO_CONFIG.topics);
    execSync(`gh repo edit ${REPO_CONFIG.org}/${REPO_CONFIG.name} --add-topic "${REPO_CONFIG.topics.join(',')}"`, {
      stdio: 'inherit'
    });
    log('✅ Topics added', 'green');

    // Disable wiki and projects
    execSync(`gh repo edit ${REPO_CONFIG.org}/${REPO_CONFIG.name} --enable-issues --enable-downloads`, {
      stdio: 'inherit'
    });
    log('✅ Repository settings configured', 'green');

    // Generate initial README
    log('\nGenerating initial README...', 'cyan');
    try {
      execSync('node scripts/generate-readme.js', {
        cwd: rootDir,
        stdio: 'inherit'
      });
      log('✅ README generated', 'green');
    } catch (error) {
      log('⚠️  README generation failed, will be created on first release', 'yellow');
    }

    // Summary
    log('\n════════════════════════════════════════', 'cyan');
    log('  Repository Setup Complete! 🎉', 'green');
    log('════════════════════════════════════════\n', 'cyan');

    log('Repository Details:', 'cyan');
    log(`  📦 Name: ${REPO_CONFIG.org}/${REPO_CONFIG.name}`, 'reset');
    log(`  🌐 URL: https://github.com/${REPO_CONFIG.org}/${REPO_CONFIG.name}`, 'reset');
    log(`  📝 Description: ${REPO_CONFIG.description}`, 'reset');
    log(`  🏷️  Topics: ${REPO_CONFIG.topics.join(', ')}`, 'reset');
    
    log('\nNext Steps:', 'cyan');
    log('  1. Run: bun run release', 'yellow');
    log('  2. First release will populate the repository', 'reset');
    log('  3. Users can download from releases page', 'reset');
    log('  4. Auto-updates will work from this repo\n', 'reset');

  } catch (error) {
    log(`\n❌ Error: ${error.message}`, 'red');
    log('\nMake sure you have GitHub CLI installed and authenticated:', 'yellow');
    log('  gh auth login\n', 'yellow');
    process.exit(1);
  }
}

createRepository();

