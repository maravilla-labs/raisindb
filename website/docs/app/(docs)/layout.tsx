import { DocsSidebar } from '@/components/DocsSidebar';

export default function DocsLayout({ children }: { children: React.ReactNode }) {
  return (
    <div className="relative flex gap-8">
      <DocsSidebar />
      <div className="flex-1 space-y-12">{children}</div>
    </div>
  );
}
