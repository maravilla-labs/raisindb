'use client';

import Link from 'next/link';
import { usePathname } from 'next/navigation';
import clsx from 'clsx';
import { docsNav } from '@/content/navigation';

export function DocsSidebar() {
  const pathname = usePathname();

  return (
    <aside className="sticky top-28 hidden h-[calc(100vh-8rem)] w-64 flex-shrink-0 flex-col overflow-y-auto rounded-2xl border border-white/10 bg-black/40 px-5 py-6 backdrop-blur-xl lg:flex">
      <p className="text-xs uppercase tracking-[0.3em] text-raisin-200">Documentation</p>
      <p className="mt-2 text-sm text-slate-400">Explore RaisinDB from concept to APIs.</p>
      <div className="mt-6 space-y-6">
        {docsNav.map((section) => (
          <div key={section.title}>
            <p className="text-xs font-semibold uppercase tracking-[0.25em] text-slate-400">{section.title}</p>
            <ul className="mt-3 space-y-2">
              {section.links.map((link) => {
                const active = pathname === link.href;
                return (
                  <li key={link.href}>
                    <Link
                      href={link.href}
                      className={clsx(
                        'block rounded-xl border px-4 py-3 transition-all',
                        active
                          ? 'border-white/60 bg-white/90 text-slate-900'
                          : 'border-white/10 bg-white/5 text-slate-200 hover:border-white/30 hover:bg-white/10'
                      )}
                    >
                      <div className="text-sm font-semibold">{link.label}</div>
                      <p className="mt-1 text-xs text-slate-400">{link.description}</p>
                    </Link>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
      </div>
    </aside>
  );
}
