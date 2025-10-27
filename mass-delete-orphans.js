#!/usr/bin/env node

import { createDirectus, rest, staticToken, readFiles, deleteFiles } from '@directus/sdk';

// Directus configuration from MCP
const DIRECTUS_URL = 'https://server.battles.app';
const ADMIN_TOKEN = 'AhY_g2PTZe5lyMRSzpJ_hzOy_nOBPAQB';

async function getOrphanedFileIds(client) {
  try {
    const files = await client.request(
      readFiles({
        limit: -1,
        fields: ['id', 'filename_disk', 'filename_download']
      })
    );
    return files;
  } catch (error) {
    throw error;
  }
}

async function massDeleteFiles(client, fileIds, batchSize = 50) {
  let totalDeleted = 0;
  let totalErrors = 0;
  const errors = [];
  
  // Process in batches
  for (let i = 0; i < fileIds.length; i += batchSize) {
    const batch = fileIds.slice(i, Math.min(i + batchSize, fileIds.length));
    const batchNum = Math.floor(i / batchSize) + 1;
    const totalBatches = Math.ceil(fileIds.length / batchSize);
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
    // Small delay between batches to avoid rate limiting
    if (i + batchSize < fileIds.length) {
      await new Promise(resolve => setTimeout(resolve, 100));
    }
  }
  
  return { totalDeleted, totalErrors, errors };
}

async function deleteByFilenames(client, filenames) {
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
    if (notFound.length > 0) {
    }
    return fileIds;
  } catch (error) {
    throw error;
  }
}

async function main() {
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
    const client = createDirectus(DIRECTUS_URL)
      .with(rest())
      .with(staticToken(ADMIN_TOKEN));
    if (mode === 'filenames') {
      const filenamesIndex = args.indexOf('--filenames');
      const filenamesString = args[filenamesIndex + 1];
      const filenames = filenamesString.split(',').map(f => f.trim());
      
      fileIds = await deleteByFilenames(client, filenames);
    } else if (mode === 'check-all') {
      process.exit(1);
    } else if (mode === 'prompt') {
      process.exit(1);
    }
    
    if (fileIds.length === 0) {
      process.exit(0);
    }
    await new Promise(resolve => setTimeout(resolve, 5000));
    
    // Perform mass deletion
    const result = await massDeleteFiles(client, fileIds);
    if (result.errors.length > 0) {
      result.errors.slice(0, 20).forEach((err, i) => {
      });
      
      if (result.errors.length > 20) {
      }
    }
  } catch (error) {
    if (error.stack) {
    }
    process.exit(1);
  }
}

main();

