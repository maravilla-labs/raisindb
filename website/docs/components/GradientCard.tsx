import { ReactNode } from 'react';
import clsx from 'clsx';

interface GradientCardProps {
  title: string;
  description: string;
  badge?: string;
  icon?: ReactNode;
  children?: ReactNode;
  className?: string;
}

export function GradientCard({ title, description, badge, icon, children, className }: GradientCardProps) {
  return (
    <div
      className={clsx(
        'group relative flex h-full flex-col gap-4 rounded-3xl border border-white/10 bg-gradient-to-br from-white/10 via-raisin-900/30 to-black/80 p-6 text-left shadow-xl shadow-black/40 transition hover:-translate-y-1',
        className
      )}
    >
      <div className="flex items-center gap-3 text-sm uppercase tracking-[0.2em] text-raisin-200">
        {icon}
        {badge && <span>{badge}</span>}
      </div>
      <div>
        <h3 className="text-xl font-semibold text-white">{title}</h3>
        <p className="mt-2 text-sm text-slate-300">{description}</p>
      </div>
      {children}
      <div className="absolute inset-0 -z-10 rounded-3xl bg-gradient-to-r from-raisin-500/20 to-transparent opacity-0 transition-opacity duration-500 group-hover:opacity-100" />
    </div>
  );
}
