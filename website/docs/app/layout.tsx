import type { Metadata } from 'next';
import { Inter, IBM_Plex_Mono } from 'next/font/google';
import './globals.css';
import { Navigation } from '@/components/Navigation';
import { Footer } from '@/components/Footer';

const inter = Inter({ subsets: ['latin'], variable: '--font-sans' });
const plex = IBM_Plex_Mono({ subsets: ['latin'], weight: ['400', '500'], variable: '--font-mono' });

export const metadata: Metadata = {
  title: 'RaisinDB Documentation',
  description: 'End-user documentation generated from the live RaisinDB client SDK and transport services.',
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={`${inter.variable} ${plex.variable}`}>
      <body className="min-h-screen bg-black text-white">
        <div className="mx-auto flex min-h-screen w-full max-w-6xl flex-col px-4 pb-12">
          <Navigation />
          <main className="flex-1 py-8">{children}</main>
          <Footer />
        </div>
      </body>
    </html>
  );
}
