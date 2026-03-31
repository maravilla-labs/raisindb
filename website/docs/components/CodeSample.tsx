'use client';

import { useEffect, useId } from 'react';
import hljs from 'highlight.js/lib/core';
import typescript from 'highlight.js/lib/languages/typescript';
import bash from 'highlight.js/lib/languages/bash';
import json from 'highlight.js/lib/languages/json';
import yaml from 'highlight.js/lib/languages/yaml';
import sql from 'highlight.js/lib/languages/sql';
import clsx from 'clsx';

hljs.registerLanguage('typescript', typescript);
hljs.registerLanguage('bash', bash);
hljs.registerLanguage('json', json);
hljs.registerLanguage('yaml', yaml);
hljs.registerLanguage('sql', sql);

interface CodeSampleProps {
  code: string;
  language?: 'typescript' | 'bash' | 'json' | 'yaml' | 'sql';
  title?: string;
  caption?: string;
}

export function CodeSample({ code, language = 'typescript', title, caption }: CodeSampleProps) {
  const codeId = useId();

  useEffect(() => {
    const block = document.getElementById(codeId);
    if (block) {
      hljs.highlightElement(block as HTMLElement);
    }
  }, [codeId, code]);

  return (
    <div className="rounded-2xl border border-white/10 bg-black/60">
      <div className="flex items-center justify-between border-b border-white/5 px-4 py-3 text-xs uppercase tracking-[0.3em] text-slate-400">
        <span>{title ?? `${language.toUpperCase()} EXAMPLE`}</span>
        {caption && <span className="text-[10px] text-slate-500">{caption}</span>}
      </div>
      <div className="code-scrollbar overflow-auto p-4">
        <pre className={clsx('text-sm leading-relaxed text-slate-100')}>
          <code id={codeId} className={`language-${language}`}>
            {code}
          </code>
        </pre>
      </div>
    </div>
  );
}
