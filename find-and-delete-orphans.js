#!/usr/bin/env node

import { createDirectus, rest, staticToken } from '@directus/sdk';

// Directus configuration from MCP
const DIRECTUS_URL = 'https://tiktok.b4battle.com';
const ADMIN_TOKEN = 'EovADLTikaBesWVpHxZb1vy5m6GTXatL';

async function findOrphanedFiles(client) {
  console.log('🔍 Querying database for orphaned files...\n');
  console.log('   Finding directus_files entries where files don\'t exist on disk...\n');
  
  try {
    // Query to find orphaned files - entries in directus_files that don't exist in fs_files
    const query = `
      SELECT id, filename_disk, filename_download, filesize, uploaded_on
      FROM directus_files
      WHERE filename_disk NOT IN (SELECT name FROM fs_files)
      ORDER BY uploaded_on DESC
    `;
    
    const response = await fetch(`${DIRECTUS_URL}/items/directus_files?filter[filename_disk][_nnull]=true&limit=-1`, {
      method: 'GET',
      headers: {
        'Authorization': `Bearer ${ADMIN_TOKEN}`,
        'Content-Type': 'application/json'
      }
    });
    
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}: ${await response.text()}`);
    }
    
    const allFiles = await response.json();
    
    console.log(`✅ Found ${allFiles.data.length} files in directus_files\n`);
    
    // Now we need to check which ones exist on disk
    // We'll do this by trying to access each file via the assets endpoint
    console.log('🔍 Checking file accessibility...\n');
    
    const orphanedFiles = [];
    const batchSize = 10;
    
    for (let i = 0; i < allFiles.data.length; i += batchSize) {
      const batch = allFiles.data.slice(i, Math.min(i + batchSize, allFiles.data.length));
      
      const checks = await Promise.all(batch.map(async (file) => {
        try {
          const headResponse = await fetch(`${DIRECTUS_URL}/assets/${file.id}`, {
            method: 'HEAD',
            headers: { 'Authorization': `Bearer ${ADMIN_TOKEN}` }
          });
          
          return {
            file,
            exists: headResponse.ok
          };
        } catch (error) {
          return {
            file,
            exists: false
          };
        }
      }));
      
      checks.forEach(({ file, exists }) => {
        if (!exists) {
          orphanedFiles.push(file);
          process.stdout.write('❌');
        } else {
          process.stdout.write('✅');
        }
      });
      
      const progress = Math.min(i + batchSize, allFiles.data.length);
      process.stdout.write(` ${progress}/${allFiles.data.length}\n`);
    }
    
    console.log(`\n📊 Found ${orphanedFiles.length} orphaned files\n`);
    
    return orphanedFiles;
    
  } catch (error) {
    console.error('❌ Error querying database:', error.message);
    throw error;
  }
}

async function deleteFiles(fileIds) {
  console.log(`\n🗑️  Deleting ${fileIds.length} orphaned files via Directus API...\n`);
  
  let totalDeleted = 0;
  let totalErrors = 0;
  const errors = [];
  const batchSize = 20;
  
  for (let i = 0; i < fileIds.length; i += batchSize) {
    const batch = fileIds.slice(i, Math.min(i + batchSize, fileIds.length));
    const batchNum = Math.floor(i / batchSize) + 1;
    const totalBatches = Math.ceil(fileIds.length / batchSize);
    
    console.log(`📦 Batch ${batchNum}/${totalBatches} (${batch.length} files)...`);
    
    const deletePromises = batch.map(async (fileId) => {
      try {
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
          return { success: false, fileId, error: `HTTP ${response.status}` };
        }
      } catch (error) {
        return { success: false, fileId, error: error.message };
      }
    });
    
    const results = await Promise.allSettled(deletePromises);
    
    let batchSuccess = 0;
    results.forEach((result, index) => {
      if (result.status === 'fulfilled' && result.value.success) {
        batchSuccess++;
        totalDeleted++;
        process.stdout.write('✅');
      } else {
        totalErrors++;
        const fileId = batch[index];
        const error = result.status === 'fulfilled' ? result.value.error : result.reason;
        errors.push({ id: fileId, error });
        process.stdout.write('❌');
      }
    });
    
    console.log(` | ✅ ${batchSuccess} deleted\n`);
    
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  
  return { totalDeleted, totalErrors, errors };
}

async function main() {
  console.log('╔════════════════════════════════════════════════════════════════════════╗');
  console.log('║     Directus Orphaned Files Finder & Deleter via API                  ║');
  console.log('╚════════════════════════════════════════════════════════════════════════╝\n');
  
  const dryRun = !process.argv.includes('--execute');
  
  try {
    console.log('🔌 Connecting to Directus...');
    console.log(`   URL: ${DIRECTUS_URL}`);
    const client = createDirectus(DIRECTUS_URL)
      .with(rest())
      .with(staticToken(ADMIN_TOKEN));
    
    console.log('✅ Connected to Directus\n');
    
    const orphanedFiles = await findOrphanedFiles(client);
    
    if (orphanedFiles.length === 0) {
      console.log('🎉 No orphaned files found! Database is clean.\n');
      return;
    }
    
    console.log('📋 Orphaned files:');
    console.log('─'.repeat(80));
    orphanedFiles.slice(0, 50).forEach((file, index) => {
      const size = file.filesize ? `${(file.filesize / 1024).toFixed(2)} KB` : 'unknown';
      console.log(`${index + 1}. ${file.filename_download || file.filename_disk}`);
      console.log(`   ID: ${file.id}`);
      console.log(`   Size: ${size}`);
      console.log(`   Uploaded: ${file.uploaded_on || 'unknown'}\n`);
    });
    
    if (orphanedFiles.length > 50) {
      console.log(`   ... and ${orphanedFiles.length - 50} more files\n`);
    }
    
    if (dryRun) {
      console.log('\n⚠️  DRY RUN MODE - No files will be deleted\n');
      console.log('💡 To execute deletion, run:');
      console.log('   node find-and-delete-orphans.js --execute\n');
      return;
    }
    
    console.log('\n⚠️  WARNING: About to delete these files via Directus API!\n');
    console.log('Press Ctrl+C to cancel, or wait 5 seconds to continue...\n');
    await new Promise(resolve => setTimeout(resolve, 5000));
    
    const fileIds = orphanedFiles.map(f => f.id);
    const result = await deleteFiles(fileIds);
    
    console.log('\n' + '═'.repeat(80));
    console.log('\n📊 Deletion Summary:');
    console.log(`   ✅ Successfully deleted: ${result.totalDeleted}`);
    console.log(`   ❌ Errors: ${result.totalErrors}\n`);
    
    if (result.errors.length > 0) {
      console.log('❌ Errors encountered:');
      console.log('─'.repeat(80));
      result.errors.slice(0, 20).forEach((err, i) => {
        console.log(`${i + 1}. File ID: ${err.id} - ${err.error}`);
      });
      
      if (result.errors.length > 20) {
        console.log(`   ... and ${result.errors.length - 20} more errors\n`);
      }
    }
    
    console.log('\n✅ Cleanup completed!\n');
    
  } catch (error) {
    console.error('\n❌ Fatal error:', error.message);
    if (error.stack) {
      console.error('\nStack trace:', error.stack);
    }
    process.exit(1);
  }
}

main();

