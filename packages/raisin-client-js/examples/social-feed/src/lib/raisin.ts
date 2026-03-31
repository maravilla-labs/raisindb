import { RaisinClient } from '@raisindb/client';

// Get RaisinDB URL from environment (works in both Node.js and browser contexts)
const getRaisinUrl = () => {
  // Node.js context (tsx/init-db script)
  if (typeof process !== 'undefined' && process.env?.VITE_RAISIN_URL) {
    return process.env.VITE_RAISIN_URL;
  }
  // Browser context (Vite)
  if (typeof import.meta.env !== 'undefined' && import.meta.env?.VITE_RAISIN_URL) {
    return import.meta.env.VITE_RAISIN_URL;
  }
  // Default fallback
  return 'ws://localhost:8081/sys/default';
};

// Configuration for local RaisinDB instance
const CONFIG = {
  url: getRaisinUrl(),
  tenant: 'default',
  credentials: {
    username: 'admin',
    password: 'Admin1234567!8',
  },
  repository: 'social_feed_demo_rel4',
  workspace: 'social',
};

// Singleton client instance
let client: RaisinClient | null = null;

export async function getClient(): Promise<RaisinClient> {
  console.log('🔍 Retrieving RaisinDB client...');
  if (client) {
    console.log('✅ Existing client found\n');
    return client;
  }
  console.log('⚙️  Creating new RaisinDB client...');

  client = new RaisinClient(CONFIG.url, {
    tenantId: CONFIG.tenant,
    defaultBranch: 'main',
  });
  console.log('🔌 Connecting to RaisinDB...');
  await client.connect();
  console.log('🔑 Authenticating with RaisinDB...');
  await client.authenticate(CONFIG.credentials);
  console.log("✅ Connected and authenticated with RaisinDB\n");
  return client;
}

export function getConfig() {
  return CONFIG;
}

export async function disconnect() {
  if (client) {
    client.disconnect();
    client = null;
  }
}
