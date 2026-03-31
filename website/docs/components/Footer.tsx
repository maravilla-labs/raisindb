export function Footer() {
  return (
    <footer className="mx-auto mt-16 w-full max-w-6xl px-6 pb-10 text-sm text-slate-400">
      <div className="section-shell flex flex-col gap-3 bg-black/40 px-6 py-6 text-center text-slate-300">
        <p>RaisinDB documentation is generated directly from live client and transport implementations.</p>
        <p className="text-xs text-slate-500">
          Built with Next.js, Tailwind CSS, and the official Raisin client APIs.
        </p>
      </div>
    </footer>
  );
}
