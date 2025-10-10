#!/usr/bin/env node

import { createDirectus, rest, staticToken, readFiles, deleteFiles } from '@directus/sdk';

// Directus configuration from MCP
const DIRECTUS_URL = 'https://tiktok.b4battle.com';
const ADMIN_TOKEN = 'EovADLTikaBesWVpHxZb1vy5m6GTXatL';

async function getOrphanedFileIds(client) {
  console.log('📊 Fetching all files from Directus database...\n');
  
  try {
    const files = await client.request(
      readFiles({
        limit: -1,
        fields: ['id', 'filename_disk', 'filename_download']
      })
    );
    
    console.log(`✅ Found ${files.length} files in database\n`);
    return files;
  } catch (error) {
    console.error('❌ Error fetching files:', error.message);
    throw error;
  }
}

async function massDeleteFiles(client, fileIds, batchSize = 50) {
  console.log(`\n🗑️  Mass deleting ${fileIds.length} orphaned files...\n`);
  console.log(`   Batch size: ${batchSize} files per request\n`);
  
  let totalDeleted = 0;
  let totalErrors = 0;
  const errors = [];
  
  // Process in batches
  for (let i = 0; i < fileIds.length; i += batchSize) {
    const batch = fileIds.slice(i, Math.min(i + batchSize, fileIds.length));
    const batchNum = Math.floor(i / batchSize) + 1;
    const totalBatches = Math.ceil(fileIds.length / batchSize);
    
    console.log(`📦 Processing batch ${batchNum}/${totalBatches} (${batch.length} files)...`);
    
    // Delete files one by one but with Promise.allSettled for parallel execution
    const deletePromises = batch.map(async (fileId) => {
      try {
        // Use the REST endpoint directly for deletion
        const response = await fetch(`${DIRECTUS_URL}/files/${fileId}`, {
          method: 'DELETE',
          headers: {
            'Authorization': `Bearer ${ADMIN_TOKEN}`,
            'Content-Type': 'application/json'
          }
        });
        
        if (response.ok || response.status === 204) {
          return { success: true, fileId };
        } else {
          const errorText = await response.text();
          return { success: false, fileId, error: `HTTP ${response.status}: ${errorText}` };
        }
      } catch (error) {
        return { success: false, fileId, error: error.message };
      }
    });
    
    const results = await Promise.allSettled(deletePromises);
    
    // Process results
    let batchSuccess = 0;
    let batchErrors = 0;
    
    results.forEach((result, index) => {
      if (result.status === 'fulfilled') {
        if (result.value.success) {
          batchSuccess++;
          totalDeleted++;
          process.stdout.write('✅');
        } else {
          batchErrors++;
          totalErrors++;
          errors.push({ id: result.value.fileId, error: result.value.error });
          process.stdout.write('❌');
        }
      } else {
        batchErrors++;
        totalErrors++;
        errors.push({ id: batch[index], error: result.reason });
        process.stdout.write('❌');
      }
    });
    
    console.log(` | ✅ ${batchSuccess} deleted, ❌ ${batchErrors} errors`);
    
    // Small delay between batches to avoid rate limiting
    if (i + batchSize < fileIds.length) {
      await new Promise(resolve => setTimeout(resolve, 100));
    }
  }
  
  return { totalDeleted, totalErrors, errors };
}

async function deleteByFilenames(client, filenames) {
  console.log('🔍 Finding file IDs from filenames...\n');
  
  try {
    const allFiles = await client.request(
      readFiles({
        limit: -1,
        fields: ['id', 'filename_disk']
      })
    );
    
    // Create a map of filename_disk to id
    const filenameMap = new Map(allFiles.map(f => [f.filename_disk, f.id]));
    
    // Find IDs for the given filenames
    const fileIds = [];
    const notFound = [];
    
    for (const filename of filenames) {
      const id = filenameMap.get(filename);
      if (id) {
        fileIds.push(id);
      } else {
        notFound.push(filename);
      }
    }
    
    console.log(`✅ Found ${fileIds.length} matching files`);
    if (notFound.length > 0) {
      console.log(`⚠️  ${notFound.length} filenames not found in database`);
    }
    console.log('');
    
    return fileIds;
  } catch (error) {
    console.error('❌ Error finding files:', error.message);
    throw error;
  }
}

async function main() {
  console.log('╔════════════════════════════════════════════════════════════════════════╗');
  console.log('║         Directus Mass Delete - Orphaned Files Remover                 ║');
  console.log('╚════════════════════════════════════════════════════════════════════════╝\n');
  
  const args = process.argv.slice(2);
  
  // Check for file list
  let fileIds = [];
  let mode = 'prompt';
  
  if (args.includes('--ids')) {
    // IDs provided as comma-separated list
    const idsIndex = args.indexOf('--ids');
    const idsString = args[idsIndex + 1];
    fileIds = idsString.split(',').map(id => id.trim());
    mode = 'ids';
  } else if (args.includes('--filenames')) {
    // Filenames provided as comma-separated list
    mode = 'filenames';
  } else if (args.includes('--all-orphans')) {
    // Will check all files for orphans
    mode = 'check-all';
  }
  
  try {
    // Create Directus client
    console.log('🔌 Connecting to Directus...');
    console.log(`   URL: ${DIRECTUS_URL}`);
    const client = createDirectus(DIRECTUS_URL)
      .with(rest())
      .with(staticToken(ADMIN_TOKEN));
    
    console.log('✅ Connected to Directus\n');
    
    if (mode === 'filenames') {
      const filenamesIndex = args.indexOf('--filenames');
      const filenamesString = args[filenamesIndex + 1];
      const filenames = filenamesString.split(',').map(f => f.trim());
      
      fileIds = await deleteByFilenames(client, filenames);
    } else if (mode === 'check-all') {
      console.log('⚠️  This mode is not yet implemented.');
      console.log('💡 Please use --ids or --filenames to specify files to delete.\n');
      process.exit(1);
    } else if (mode === 'prompt') {
      console.log('❌ No files specified for deletion.\n');
      console.log('Usage:');
      console.log('  node mass-delete-orphans.js --ids "id1,id2,id3"');
      console.log('  node mass-delete-orphans.js --filenames "file1.jpg,file2.jpg"');
      console.log('  node mass-delete-orphans.js --all-orphans\n');
      process.exit(1);
    }
    
    if (fileIds.length === 0) {
      console.log('❌ No files to delete.\n');
      process.exit(0);
    }
    
    console.log(`📋 Files to delete: ${fileIds.length}\n`);
    console.log('⚠️  WARNING: This will permanently delete these files from Directus!\n');
    console.log('Press Ctrl+C to cancel, or wait 5 seconds to continue...\n');
    
    await new Promise(resolve => setTimeout(resolve, 5000));
    
    // Perform mass deletion
    const result = await massDeleteFiles(client, fileIds);
    
    console.log('\n' + '═'.repeat(80));
    console.log('\n📊 Mass Deletion Summary:');
    console.log(`   ✅ Successfully deleted: ${result.totalDeleted}`);
    console.log(`   ❌ Errors: ${result.totalErrors}\n`);
    
    if (result.errors.length > 0) {
      console.log('❌ Errors encountered:');
      console.log('─'.repeat(80));
      result.errors.slice(0, 20).forEach((err, i) => {
        console.log(`${i + 1}. File ID: ${err.id}`);
        console.log(`   Error: ${err.error}\n`);
      });
      
      if (result.errors.length > 20) {
        console.log(`   ... and ${result.errors.length - 20} more errors\n`);
      }
    }
    
    console.log('✅ Mass deletion completed!\n');
    
  } catch (error) {
    console.error('\n❌ Fatal error:', error.message);
    if (error.stack) {
      console.error('\nStack trace:', error.stack);
    }
    process.exit(1);
  }
}

main();

