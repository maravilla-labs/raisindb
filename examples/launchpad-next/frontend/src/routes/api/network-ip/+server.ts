import { json } from '@sveltejs/kit';
import type { RequestHandler } from './$types';
import { networkInterfaces } from 'os';

export const GET: RequestHandler = async () => {
  const interfaces = networkInterfaces();
  const addresses: string[] = [];

  for (const name of Object.keys(interfaces)) {
    const nets = interfaces[name];
    if (!nets) continue;

    for (const net of nets) {
      // Skip internal (loopback) addresses
      if (net.internal) continue;

      // Only IPv4 addresses
      if (net.family === 'IPv4') {
        addresses.push(net.address);
      }
    }
  }

  // Prefer addresses that look like local network IPs
  const preferredAddress = addresses.find(
    (addr) =>
      addr.startsWith('192.168.') ||
      addr.startsWith('10.') ||
      addr.startsWith('172.')
  ) || addresses[0] || null;

  return json({
    addresses,
    preferred: preferredAddress,
    port: 5173
  });
};
