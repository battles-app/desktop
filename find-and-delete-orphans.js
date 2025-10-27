#!/usr/bin/env node

import { createDirectus, rest, staticToken } from '@directus/sdk';

// Directus configuration from MCP
const DIRECTUS_URL = 'https://server.battles.app';
const ADMIN_TOKEN = 'AhY_g2PTZe5lyMRSzpJ_hzOy_nOBPAQB';

async function findOrphanedFiles(client) {
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
    // Now we need to check which ones exist on disk
    // We'll do this by trying to access each file via the assets endpoint
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
    return orphanedFiles;
    
  } catch (error) {
    throw error;
  }
}

async function deleteFiles(fileIds) {
  let totalDeleted = 0;
  let totalErrors = 0;
  const errors = [];
  const batchSize = 20;
  
  for (let i = 0; i < fileIds.length; i += batchSize) {
    const batch = fileIds.slice(i, Math.min(i + batchSize, fileIds.length));
    const batchNum = Math.floor(i / batchSize) + 1;
    const totalBatches = Math.ceil(fileIds.length / batchSize);
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
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  
  return { totalDeleted, totalErrors, errors };
}

async function main() {
  const dryRun = !process.argv.includes('--execute');
  
  try {
    const client = createDirectus(DIRECTUS_URL)
      .with(rest())
      .with(staticToken(ADMIN_TOKEN));
    const orphanedFiles = await findOrphanedFiles(client);
    
    if (orphanedFiles.length === 0) {
      return;
    }
    orphanedFiles.slice(0, 50).forEach((file, index) => {
      const size = file.filesize ? `${(file.filesize / 1024).toFixed(2)} KB` : 'unknown';
    });
    
    if (orphanedFiles.length > 50) {
    }
    
    if (dryRun) {
      return;
    }
    await new Promise(resolve => setTimeout(resolve, 5000));
    
    const fileIds = orphanedFiles.map(f => f.id);
    const result = await deleteFiles(fileIds);
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

