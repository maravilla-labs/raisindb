import type { Pack } from '../types.js';
import { contentModelingPack } from './content-modeling.js';
import { minimalPack } from './minimal.js';

const packs: Record<string, Pack> = {
  'minimal': minimalPack,
  'content-modeling': contentModelingPack,
};

export function getPack(name: string): Pack {
  const pack = packs[name];
  if (!pack) {
    const available = Object.keys(packs).join(', ');
    throw new Error(`Unknown pack "${name}". Available packs: ${available}`);
  }
  return pack;
}
