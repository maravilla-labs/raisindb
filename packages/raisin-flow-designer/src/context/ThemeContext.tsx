/**
 * Theme Context for Flow Designer
 *
 * Provides theme configuration to all child components.
 */

import { createContext, useContext, type ReactNode } from 'react';

export type FlowTheme = 'light' | 'dark';

interface ThemeContextValue {
  theme: FlowTheme;
}

const ThemeContext = createContext<ThemeContextValue>({ theme: 'light' });

export interface ThemeProviderProps {
  theme: FlowTheme;
  children: ReactNode;
}

export function ThemeProvider({ theme, children }: ThemeProviderProps) {
  return (
    <ThemeContext.Provider value={{ theme }}>
      {children}
    </ThemeContext.Provider>
  );
}

export function useTheme(): ThemeContextValue {
  return useContext(ThemeContext);
}

/**
 * Theme-aware class helper
 * Returns light or dark classes based on current theme
 */
export function useThemeClasses() {
  const { theme } = useTheme();
  const isDark = theme === 'dark';

  return {
    theme,
    isDark,
    // Canvas
    canvasBg: isDark
      ? 'bg-[#1a1a2e]'
      : 'bg-gray-50',
    canvasDots: isDark
      ? 'bg-[radial-gradient(circle,_rgba(255,255,255,0.08)_1px,_transparent_1px)]'
      : 'bg-[radial-gradient(circle,_rgba(0,0,0,0.08)_1px,_transparent_1px)]',
    // Step node
    stepBg: isDark ? 'bg-gray-800' : 'bg-white',
    stepBgDisabled: isDark ? 'bg-gray-900' : 'bg-gray-50',
    stepText: isDark ? 'text-white' : 'text-gray-900',
    stepTextMuted: isDark ? 'text-gray-400' : 'text-gray-500',
    stepTextFaint: isDark ? 'text-gray-500' : 'text-gray-400',
    stepBorder: isDark ? 'border-white/10' : 'border-gray-200',
    stepShadow: isDark ? 'shadow-lg shadow-black/20' : 'shadow-lg',
    stepOutline: isDark ? 'outline-blue-400' : 'outline-blue-300',
    stepOutlineHover: isDark ? 'hover:outline-blue-500' : 'hover:outline-blue-400',
    // Start/End nodes
    startBg: isDark ? 'bg-blue-900/50' : 'bg-blue-100',
    startBgHover: isDark ? 'hover:bg-blue-800/50' : 'hover:bg-blue-200',
    startBorder: isDark ? 'border-blue-500/50' : 'border-blue-300',
    startText: isDark ? 'text-blue-300' : 'text-blue-700',
    endBg: isDark ? 'bg-green-900/50' : 'bg-green-100',
    endBgHover: isDark ? 'hover:bg-green-800/50' : 'hover:bg-green-200',
    endBorder: isDark ? 'border-green-500/50' : 'border-green-300',
    endText: isDark ? 'text-green-300' : 'text-green-700',
    // Connector
    connectorBg: isDark ? 'bg-blue-400/50' : 'bg-blue-200',
    // Empty state
    emptyBorder: isDark ? 'border-white/20' : 'border-gray-200',
    emptyBg: isDark ? 'bg-gray-800/50' : 'bg-white',
    emptyText: isDark ? 'text-gray-400' : 'text-gray-400',
    // Button colors
    btnText: isDark ? 'text-gray-300' : 'text-gray-600',
    btnHover: isDark ? 'hover:text-white hover:bg-white/10' : 'hover:text-gray-900 hover:bg-gray-100',
    // Add button in connectors
    addBtnBg: isDark ? 'bg-gray-800 border-white/20' : 'bg-white border-gray-200',
    addBtnHover: isDark ? 'hover:bg-gray-700' : 'hover:bg-gray-50',
    addBtnText: isDark ? 'text-gray-400 hover:text-white' : 'text-gray-400 hover:text-gray-600',
  };
}
