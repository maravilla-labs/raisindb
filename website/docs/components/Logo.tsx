import Image from 'next/image';
import Link from 'next/link';
import clsx from 'clsx';

interface LogoProps {
  className?: string;
  href?: string;
}

export function Logo({ className, href = '/' }: LogoProps) {
  return (
    <Link href={href} className={clsx('flex items-center gap-3 group', className)}>
      <div className="relative h-12 w-12 overflow-hidden rounded-2xl bg-raisin-500/20 p-2 transition-all duration-300 group-hover:bg-raisin-400/30">
        <Image src="/img/raisin-logo.png" alt="RaisinDB" fill sizes="48px" className="object-contain" />
      </div>
      <div className="flex flex-col leading-tight">
        <span className="text-lg font-semibold tracking-wide">RaisinDB</span>
        <span className="text-xs uppercase text-slate-400">Multi-model Platform</span>
      </div>
    </Link>
  );
}
