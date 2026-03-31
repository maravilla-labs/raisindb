import { ReactNode } from 'react';
import clsx from 'clsx';

interface PageShellProps {
  title: string;
  subtitle?: string;
  eyebrow?: string;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}

export function PageShell({ title, subtitle, eyebrow, actions, children, className }: PageShellProps) {
  return (
    <section className={clsx('section-shell mx-auto mt-12 max-w-6xl px-8 py-10', className)}>
      <div className="flex flex-col gap-4 md:flex-row md:items-center md:justify-between">
        <div>
          {eyebrow && <p className="text-xs uppercase tracking-[0.3em] text-raisin-300">{eyebrow}</p>}
          <h1 className="text-3xl font-semibold text-white md:text-4xl">{title}</h1>
          {subtitle && <p className="mt-2 text-lg text-slate-300">{subtitle}</p>}
        </div>
        {actions && <div className="flex-shrink-0">{actions}</div>}
      </div>
      <div className="mt-10 space-y-10 text-slate-100">{children}</div>
    </section>
  );
}
