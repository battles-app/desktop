#!/usr/bin/env node

import { createDirectus, rest, staticToken, readFiles, deleteFile } from '@directus/sdk';
import https from 'https';
import http from 'http';

// Directus configuration from MCP
const DIRECTUS_URL = 'https://server.battles.app';
const ADMIN_TOKEN = 'AhY_g2PTZe5lyMRSzpJ_hzOy_nOBPAQB';

async function checkFileExists(fileId) {
  return new Promise((resolve) => {
    const url = `${DIRECTUS_URL}/assets/${fileId}`;
    const protocol = url.startsWith('https') ? https : http;
    
    const options = {
      method: 'HEAD',
      headers: {
        'Authorization': `Bearer ${ADMIN_TOKEN}`
      }
    };
    
    const req = protocol.request(url, options, (res) => {
      resolve(res.statusCode === 200);
    });
    
    req.on('error', () => {
      resolve(false);
    });
    
    req.setTimeout(5000, () => {
      req.destroy();
      resolve(false);
    });
    
    req.end();
  });
}

async function getAllFilesFromDatabase(client) {
  try {
    const files = await client.request(
      readFiles({
        limit: -1, // Get all files
        fields: ['id', 'filename_disk', 'filename_download', 'title', 'type', 'filesize', 'uploaded_on', 'folder']
      })
    );
    return files;
  } catch (error) {
    if (error.errors) {
    }
    throw error;
  }
}

async function checkAllFiles(dbFiles) {
  const orphanedFiles = [];
  const validFiles = [];
  const batchSize = 10;
  
  for (let i = 0; i < dbFiles.length; i += batchSize) {
    const batch = dbFiles.slice(i, Math.min(i + batchSize, dbFiles.length));
    const checks = batch.map(async (dbFile) => {
      const exists = await checkFileExists(dbFile.id);
      return { file: dbFile, exists };
    });
    
    const results = await Promise.all(checks);
    
    for (const { file, exists } of results) {
      if (exists) {
        validFiles.push(file);
        process.stdout.write('✅');
      } else {
        orphanedFiles.push(file);
        process.stdout.write('❌');
      }
    }
    
    const progress = Math.min(i + batchSize, dbFiles.length);
    process.stdout.write(` ${progress}/${dbFiles.length}\n`);
  }
  
  return { orphanedFiles, validFiles };
}

async function cleanupOrphanedEntries(client, dbFiles, dryRun = true) {
  const { orphanedFiles, validFiles } = await checkAllFiles(dbFiles);
  if (orphanedFiles.length === 0) {
    return { deleted: 0, errors: 0 };
  }
  orphanedFiles.forEach((file, index) => {
    const size = file.filesize ? `${(file.filesize / 1024).toFixed(2)} KB` : 'unknown';
  });
  
  if (dryRun) {
    return { deleted: 0, errors: 0 };
  }
  
  // Delete orphaned entries
  let deleted = 0;
  let errors = 0;
  
  for (const file of orphanedFiles) {
    try {
      await client.request(deleteFile(file.id));
      deleted++;
    } catch (error) {
      errors++;
    }
  }
  return { deleted, errors };
}

async function main() {
  const dryRun = !process.argv.includes('--execute');
  
  try {
    // Create Directus client
    const client = createDirectus(DIRECTUS_URL)
      .with(rest())
      .with(staticToken(ADMIN_TOKEN));
    // Get all files from database
    const dbFiles = await getAllFilesFromDatabase(client);
    
    if (dbFiles.length === 0) {
      return;
    }
    
    // Find and cleanup orphaned entries
    const result = await cleanupOrphanedEntries(client, dbFiles, dryRun);
    if (dryRun && result.deleted === 0 && result.errors === 0) {
    }
    
  } catch (error) {
    if (error.stack) {
    }
    process.exit(1);
  }
}

main();

