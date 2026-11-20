import { describe, expect, it } from 'vitest';
import { buildMessageMetadata } from './types/client-tools';

describe('buildMessageMetadata', () => {
  it('keeps attachments as plain reference envelopes and moves inline image bytes to attachment_images', () => {
    // A stored reference envelope keeps only its reference keys, so bytes on it would be lost.
    const meta = buildMessageMetadata('m1', {
      attachments: [
        { 'raisin:ref': '/u/p.png', 'raisin:workspace': 'assets', mime_type: 'image/png', name: 'p.png', inline: { media_type: 'image/png', data: 'QUJD' } },
        { 'raisin:ref': '/u/a.pdf', 'raisin:workspace': 'assets', mime_type: 'application/pdf', name: 'a.pdf' },
      ],
    } as never)!;
    const atts = meta.attachments as Record<string, unknown>[];
    expect(atts.some((a) => 'inline' in a)).toBe(false);
    expect(atts[0]['raisin:ref']).toBe('/u/p.png');
    expect(meta.attachment_images).toEqual([{ path: '/u/p.png', name: 'p.png', media_type: 'image/png', data: 'QUJD' }]);
  });

  it('adds no attachment_images when nothing is inline', () => {
    const meta = buildMessageMetadata('m1', { attachments: [{ 'raisin:ref': '/u/a.pdf', 'raisin:workspace': 'assets', mime_type: 'application/pdf', name: 'a.pdf' }] } as never)!;
    expect('attachment_images' in meta).toBe(false);
  });
});

describe('buildMessageMetadata with several pictures on one attachment', () => {
  it('sends each PDF page as its own image, named with its page, and keeps one envelope', () => {
    const meta = buildMessageMetadata('m1', {
      attachments: [
        {
          'raisin:ref': '/u/order.pdf', 'raisin:workspace': 'assets', mime_type: 'application/pdf', name: 'order.pdf',
          inline: [{ media_type: 'image/jpeg', data: 'UDE=' }, { media_type: 'image/jpeg', data: 'UDI=' }],
        },
      ],
    } as never)!;
    expect((meta.attachments as unknown[]).length).toBe(1);
    expect(meta.attachment_images).toEqual([
      { path: '/u/order.pdf', name: 'order.pdf (page 1)', media_type: 'image/jpeg', data: 'UDE=' },
      { path: '/u/order.pdf', name: 'order.pdf (page 2)', media_type: 'image/jpeg', data: 'UDI=' },
    ]);
  });
});
