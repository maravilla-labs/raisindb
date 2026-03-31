'use client';

import Link from 'next/link';
import { usePathname } from 'next/navigation';
import clsx from 'clsx';
import { Logo } from './Logo';

const links = [
  { href: '/getting-started', label: 'Getting Started' },
  { href: '/client-sdk', label: 'Client SDK' },
  { href: '/http-rest', label: 'HTTP API' },
  { href: '/websocket-streaming', label: 'Realtime' },
  { href: '/sql', label: 'SQL' },
  { href: '/multi-model', label: 'Data Model' },
];

export function Navigation() {
  const pathname = usePathname();

  return (
    <header className="sticky top-4 z-50 mx-auto w-full max-w-6xl">
      <nav className="section-shell flex items-center justify-between px-6 py-4">
        <Logo />
        <div className="hidden gap-1 md:flex">
          {links.map((link) => (
            <Link
              key={link.href}
              href={link.href}
              className={clsx(
                'rounded-full px-4 py-2 text-sm font-medium transition-all',
                pathname?.startsWith(link.href)
                  ? 'bg-white/90 text-slate-900'
                  : 'text-slate-300 hover:bg-white/10'
              )}
            >
              {link.label}
            </Link>
          ))}
        </div>
        <div className="md:hidden">
          <Link href="/getting-started" className="rounded-full bg-white/90 px-4 py-2 text-sm font-semibold text-slate-900">
            Docs
          </Link>
        </div>
      </nav>
    </header>
  );
}
