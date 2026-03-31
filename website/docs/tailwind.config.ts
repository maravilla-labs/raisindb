import type { Config } from 'tailwindcss';

const config: Config = {
  content: [
    './app/**/*.{ts,tsx}',
    './components/**/*.{ts,tsx}',
    './content/**/*.{ts,tsx,mdx}',
  ],
  theme: {
    extend: {
      colors: {
        raisin: {
          50: '#fdf8f5',
          100: '#f8ebe2',
          200: '#efd1be',
          300: '#e6b694',
          400: '#dc9060',
          500: '#c86a3a',
          600: '#a54f28',
          700: '#7e3b1f',
          800: '#592918',
          900: '#3a1b12',
        },
      },
      fontFamily: {
        sans: ['"InterVariable"', 'Inter', 'system-ui', 'sans-serif'],
        mono: ['"IBM Plex Mono"', 'monospace'],
      },
      boxShadow: {
        glow: '0 35px 120px rgba(200, 106, 58, 0.25)',
      },
      keyframes: {
        float: {
          '0%, 100%': { transform: 'translateY(0px)' },
          '50%': { transform: 'translateY(-6px)' },
        },
        pulseBorder: {
          '0%': { opacity: 0.35 },
          '50%': { opacity: 0.8 },
          '100%': { opacity: 0.35 },
        },
      },
      animation: {
        float: 'float 6s ease-in-out infinite',
        'pulse-border': 'pulseBorder 4s ease-in-out infinite',
      },
    },
  },
  plugins: [],
};

export default config;
