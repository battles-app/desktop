#!/usr/bin/env node

import { createDirectus, rest, staticToken, readFiles, deleteFile } from '@directus/sdk';
import https from 'https';
import http from 'http';

// Directus configuration from MCP
const DIRECTUS_URL = 'https://tiktok.b4battle.com';
const ADMIN_TOKEN = 'EovADLTikaBesWVpHxZb1vy5m6GTXatL';

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
  console.log('📊 Fetching all files from Directus database...\n');
  
  try {
    const files = await client.request(
      readFiles({
        limit: -1, // Get all files
        fields: ['id', 'filename_disk', 'filename_download', 'title', 'type', 'filesize', 'uploaded_on', 'folder']
      })
    );
    
    console.log(`✅ Found ${files.length} files in database\n`);
    return files;
  } catch (error) {
    console.error('❌ Error fetching files from database:', error.message);
    if (error.errors) {
      console.error('   Details:', JSON.stringify(error.errors, null, 2));
    }
    throw error;
  }
}

async function checkAllFiles(dbFiles) {
  console.log('🔍 Checking file accessibility via Directus API...\n');
  console.log(`   This may take a while for ${dbFiles.length} files...\n`);
  
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
  console.log('🧹 Analyzing orphaned database entries...\n');
  console.log(`Mode: ${dryRun ? '🔍 DRY RUN (no changes will be made)' : '⚠️  LIVE MODE (entries will be deleted)'}\n`);
  
  const { orphanedFiles, validFiles } = await checkAllFiles(dbFiles);
  
  console.log(`\n📊 Analysis Results:`);
  console.log(`   ✅ Valid entries: ${validFiles.length}`);
  console.log(`   ❌ Orphaned entries: ${orphanedFiles.length}\n`);
  
  if (orphanedFiles.length === 0) {
    console.log('🎉 No orphaned entries found! Database is clean.\n');
    return { deleted: 0, errors: 0 };
  }
  
  console.log('📋 Orphaned entries:');
  console.log('─'.repeat(80));
  orphanedFiles.forEach((file, index) => {
    const size = file.filesize ? `${(file.filesize / 1024).toFixed(2)} KB` : 'unknown';
    console.log(`${index + 1}. ${file.filename_download || file.filename_disk}`);
    console.log(`   ID: ${file.id}`);
    console.log(`   Disk filename: ${file.filename_disk}`);
    console.log(`   Size: ${size}`);
    console.log(`   Uploaded: ${file.uploaded_on || 'unknown'}`);
    console.log(`   Folder: ${file.folder || 'root'}`);
    console.log('');
  });
  
  if (dryRun) {
    console.log('\n⚠️  This is a DRY RUN. No changes were made.');
    console.log('💡 Run with --execute flag to actually delete these entries.\n');
    return { deleted: 0, errors: 0 };
  }
  
  // Delete orphaned entries
  console.log('\n🗑️  Deleting orphaned entries...\n');
  let deleted = 0;
  let errors = 0;
  
  for (const file of orphanedFiles) {
    try {
      await client.request(deleteFile(file.id));
      console.log(`✅ Deleted: ${file.filename_download || file.filename_disk} (${file.id})`);
      deleted++;
    } catch (error) {
      console.error(`❌ Failed to delete ${file.filename_disk}:`, error.message);
      errors++;
    }
  }
  
  console.log(`\n📊 Deletion Summary:`);
  console.log(`   ✅ Successfully deleted: ${deleted}`);
  console.log(`   ❌ Errors: ${errors}\n`);
  
  return { deleted, errors };
}

async function main() {
  console.log('╔════════════════════════════════════════════════════════════════════════╗');
  console.log('║       Directus Database Cleanup - Orphaned Uploads Remover            ║');
  console.log('║              Checking files via Directus API                           ║');
  console.log('╚════════════════════════════════════════════════════════════════════════╝\n');
  
  const dryRun = !process.argv.includes('--execute');
  
  try {
    // Create Directus client
    console.log('🔌 Connecting to Directus...');
    console.log(`   URL: ${DIRECTUS_URL}`);
    const client = createDirectus(DIRECTUS_URL)
      .with(rest())
      .with(staticToken(ADMIN_TOKEN));
    
    console.log('✅ Connected to Directus\n');
    
    // Get all files from database
    const dbFiles = await getAllFilesFromDatabase(client);
    
    if (dbFiles.length === 0) {
      console.log('ℹ️  No files found in database. Nothing to clean up.\n');
      return;
    }
    
    // Find and cleanup orphaned entries
    const result = await cleanupOrphanedEntries(client, dbFiles, dryRun);
    
    console.log('\n✅ Cleanup completed!\n');
    
    if (dryRun && result.deleted === 0 && result.errors === 0) {
      console.log('💡 To execute the cleanup and delete orphaned entries, run:');
      console.log('   node cleanup-directus-uploads.js --execute\n');
    }
    
  } catch (error) {
    console.error('\n❌ Fatal error:', error.message);
    if (error.stack) {
      console.error('\nStack trace:', error.stack);
    }
    process.exit(1);
  }
}

main();

