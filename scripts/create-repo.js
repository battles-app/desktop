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
}

async function createRepository() {
  try {
    // Check if repository already exists
    try {
      execSync(`gh repo view ${REPO_CONFIG.org}/${REPO_CONFIG.name}`, { 
        stdio: 'ignore' 
      });
    } catch (error) {
      // Repository doesn't exist, create it
      const createCmd = `gh repo create ${REPO_CONFIG.org}/${REPO_CONFIG.name} --public --description "${REPO_CONFIG.description}" --homepage "${REPO_CONFIG.homepage}"`;
      
      execSync(createCmd, { stdio: 'inherit' });
    }

    // Update repository settings
    // Set topics
    const topicsJson = JSON.stringify(REPO_CONFIG.topics);
    execSync(`gh repo edit ${REPO_CONFIG.org}/${REPO_CONFIG.name} --add-topic "${REPO_CONFIG.topics.join(',')}"`, {
      stdio: 'inherit'
    });
    // Disable wiki and projects
    execSync(`gh repo edit ${REPO_CONFIG.org}/${REPO_CONFIG.name} --enable-issues --enable-downloads`, {
      stdio: 'inherit'
    });
    // Generate initial README
    try {
      execSync('node scripts/generate-readme.js', {
        cwd: rootDir,
        stdio: 'inherit'
      });
    } catch (error) {
    }

    // Summary
    log(`  🏷️  Topics: ${REPO_CONFIG.topics.join(', ')}`, 'reset');
  } catch (error) {
    process.exit(1);
  }
}

createRepository();

