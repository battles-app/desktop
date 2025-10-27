#!/usr/bin/env node

/**
 * AI-Powered LICENSE Generator for Battles.app
 * 
 * Generates BSL 1.1 license with proper terms and conditions
 */

import { config } from 'dotenv';
import OpenAI from 'openai';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, '..');

// Load environment variables
config();

// Initialize OpenAI
const openai = new OpenAI({
  apiKey: process.env.OPENAI_API_KEY
});

async function generateLicense() {
  const prompt = `Generate a professional Business Source License 1.1 (BSL 1.1) for "Battles.app Desktop" software.

**Company Information:**
- Licensor: BATTLES.app™
- Copyright: © 2025 BATTLES.app. All rights reserved.
- Software: Battles.app Desktop
- Description: Professional TikTok Live streaming utilities with Elgato Stream Deck integration

**License Terms:**
- License: Business Source License 1.1 (BSL 1.1)
- Change Date: 4 years from release date
- Change License: Apache License 2.0
- Additional Use Grant: Non-production use is permitted
- Production Use: Requires separate commercial license
- API Ownership: All APIs, interfaces, and protocols are proprietary and owned by BATTLES.app™

**Requirements:**
1. Professional legal language
2. Clear definition of "Production Use" vs "Non-Production Use"
3. Explicit API ownership clause
4. Clear commercial licensing requirements
5. Attribution requirements
6. Modification restrictions
7. Distribution terms
8. Warranty disclaimers
9. Limitation of liability
10. Contact information: legal@battles.app

Generate a complete, legally sound BSL 1.1 license document. Use proper legal formatting and sections.`;

  try {
    const response = await openai.chat.completions.create({
      model: 'gpt-4-turbo-preview',
      messages: [
        {
          role: 'system',
          content: 'You are a legal document specialist who creates professional software licenses. Generate clear, legally sound licenses with proper structure and terminology.'
        },
        {
          role: 'user',
          content: prompt
        }
      ],
      temperature: 0.3, // Lower temperature for consistency
      max_tokens: 3000
    });
    
    const license = response.choices[0].message.content.trim();
    return license;
    
  } catch (error) {
    return generateFallbackLicense();
  }
}

function generateFallbackLicense() {
  return `# Business Source License 1.1

## License Grant

Licensor: BATTLES.app™  
Licensed Work: Battles.app Desktop  
Copyright: © 2025 BATTLES.app. All rights reserved.

**Change Date:** 4 years from release date  
**Change License:** Apache License 2.0

---

## Terms

The Licensor hereby grants you the right to copy, modify, create derivative works, redistribute, and make non-production use of the Licensed Work. The Licensor may make an Additional Use Grant, above, permitting limited production use.

**Production Use** means any commercial use of the software in a production environment, including but not limited to:
- Use by businesses or organizations for commercial purposes
- Integration into commercial products or services
- Use in revenue-generating activities
- Public-facing deployments serving end users

**Non-Production Use** includes:
- Personal use
- Educational purposes
- Research and development
- Testing and evaluation
- Non-commercial projects

## API Ownership

All application programming interfaces (APIs), protocols, interfaces, and integration methods provided by Battles.app Desktop are proprietary and owned exclusively by BATTLES.app™. Reverse engineering, decompilation, or extraction of APIs for use in competing products is strictly prohibited.

## Commercial License

Production use requires a separate commercial license from BATTLES.app™. For commercial licensing inquiries, contact:

**Email:** legal@battles.app  
**Website:** https://battles.app

## Restrictions

You may not:
1. Use the Licensed Work for Production Use without a commercial license
2. Remove or modify any license, copyright, or proprietary notices
3. Use BATTLES.app™ trademarks without prior written consent
4. Create derivative works that compete with the Licensed Work
5. Extract or reuse APIs in competing products

## Attribution

Any redistribution must include:
- This license file
- Copyright notices
- Attribution to BATTLES.app™
- Link to https://battles.app

## Change Date

On the Change Date, the terms of this license will automatically convert to the Change License (Apache License 2.0) for versions released before that date.

## Disclaimer

THE LICENSED WORK IS PROVIDED "AS IS" WITHOUT WARRANTIES OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE, AND NON-INFRINGEMENT.

## Limitation of Liability

IN NO EVENT SHALL THE LICENSOR BE LIABLE FOR ANY DAMAGES, INCLUDING BUT NOT LIMITED TO DIRECT, INDIRECT, INCIDENTAL, SPECIAL, CONSEQUENTIAL, OR EXEMPLARY DAMAGES ARISING OUT OF OR IN CONNECTION WITH THE USE OF THE LICENSED WORK.

---

**For the full BSL 1.1 license text, visit:** https://mariadb.com/bsl11/

**Questions? Contact:** legal@battles.app

---

© 2025 BATTLES.app™. All rights reserved.`;
}

async function main() {
  const license = await generateLicense();
  
  // Save to LICENSE file
  const licensePath = path.join(rootDir, 'LICENSE');
  fs.writeFileSync(licensePath, license, 'utf-8');
}

main().catch(error => {
  process.exit(1);
});

