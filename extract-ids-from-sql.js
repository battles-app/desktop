#!/usr/bin/env node

import { readFileSync } from 'fs';

// Usage: node extract-ids-from-sql.js < sql-output.txt
// Or: node extract-ids-from-sql.js sql-output.txt

function extractIdsFromText(text) {
  // Match UUID patterns (like 9ce80902-102f-4260-bac4-e890d9db827e)
  const uuidPattern = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi;
  
  // Match numeric IDs (like 12509451)
  const numericPattern = /\b\d{7,}\b/g;
  
  const uuids = text.match(uuidPattern) || [];
  const numericIds = text.match(numericPattern) || [];
  
  // Combine and deduplicate
  const allIds = [...new Set([...uuids, ...numericIds])];
  
  return allIds;
}

function main() {
  let input = '';
  
  if (process.argv[2]) {
    // Read from file
    try {
      input = readFileSync(process.argv[2], 'utf-8');
    } catch (error) {
      console.error(`❌ Error reading file: ${error.message}`);
      process.exit(1);
    }
  } else if (!process.stdin.isTTY) {
    // Read from stdin
    const fs = require('fs');
    input = fs.readFileSync(0, 'utf-8');
  } else {
    console.log('Usage:');
    console.log('  node extract-ids-from-sql.js < sql-output.txt');
    console.log('  node extract-ids-from-sql.js sql-output.txt');
    console.log('  cat sql-output.txt | node extract-ids-from-sql.js');
    process.exit(1);
  }
  
  const ids = extractIdsFromText(input);
  
  console.log(`Found ${ids.length} IDs:\n`);
  
  // Output as comma-separated for easy copying
  console.log('Comma-separated (for --ids flag):');
  console.log(ids.join(','));
  console.log('');
  
  // Output as JSON array
  console.log('JSON array:');
  console.log(JSON.stringify(ids, null, 2));
  console.log('');
  
  // Output command to run
  console.log('To delete these files, run:');
  console.log(`node mass-delete-orphans.js --ids "${ids.join(',')}"`);
}

main();

