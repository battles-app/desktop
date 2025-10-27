#!/usr/bin/env node

/**
 * AI-Powered Repository Metadata Updater
 * 
 * Uses OpenAI to generate professional repository descriptions,
 * about text, and other metadata for GitHub
 */

import { config } from 'dotenv';
import OpenAI from 'openai';

// Load environment variables
config();

// Initialize OpenAI
const openai = new OpenAI({
  apiKey: process.env.OPENAI_API_KEY
});

const colors = {
  reset: '\x1b[0m',
  cyan: '\x1b[36m',
  green: '\x1b[32m',
  yellow: '\x1b[33m'
};

function log(message, color = 'reset') {
}

async function generateMetadata() {
  const prompt = `Generate professional GitHub repository metadata for "Battles.app Desktop".

**Software Details:**
- Name: Battles.app Desktop
- Type: Professional TikTok Live streaming utility
- Platform: Windows 10/11 (64-bit)
- Key Features: Elgato Stream Deck integration, real-time animations, light shows, AI tools, auto-updates
- License: BSL 1.1 (Business Source License)
- Status: Closed Beta

**Generate:**

1. **Short Description** (max 350 chars for GitHub):
   - Professional, catchy, SEO-friendly
   - Highlight key value proposition
   - Include platform and main feature

2. **About/Tagline** (max 100 chars):
   - Punchy, memorable one-liner
   - What makes it unique

3. **Topics/Keywords** (up to 20 keywords):
   - SEO-optimized
   - Relevant to the software

4. **Social Preview Description** (max 200 chars):
   - Engaging description for social sharing
   - Focus on user benefits

Return as JSON with keys: shortDescription, tagline, topics (array), socialPreview`;

  try {
    const response = await openai.chat.completions.create({
      model: 'gpt-4-turbo-preview',
      messages: [
        {
          role: 'system',
          content: 'You are a professional technical writer and marketing specialist. Generate concise, impactful repository metadata that attracts users and ranks well in search. Return ONLY valid JSON.'
        },
        {
          role: 'user',
          content: prompt
        }
      ],
      temperature: 0.7,
      max_tokens: 800,
      response_format: { type: "json_object" }
    });
    
    const metadata = JSON.parse(response.choices[0].message.content.trim());
    return metadata;
    
  } catch (error) {
    return {
      shortDescription: '🎮 Professional TikTok Live streaming utilities with Elgato Stream Deck integration. Real-time animations, light shows, and AI-powered tools for engaging live streams on Windows.',
      tagline: 'Pro TikTok Live Utilities with Stream Deck Integration',
      topics: [
        'tiktok',
        'streaming',
        'elgato-streamdeck',
        'desktop-app',
        'live-streaming',
        'windows',
        'tauri',
        'rust',
        'animations',
        'light-shows',
        'ai-tools',
        'streaming-software',
        'content-creation',
        'stream-deck',
        'tiktok-live'
      ],
      socialPreview: '🎮 Transform your TikTok Live streams with professional animations, light shows, and Stream Deck control. Closed Beta now available!'
    };
  }
}

async function main() {
  const metadata = await generateMetadata();
  log(`  ${metadata.topics.join(', ')}\n`, 'reset');
  log('  3. Add topics (comma-separated)', 'reset');
}

main().catch(error => {
  process.exit(1);
});

