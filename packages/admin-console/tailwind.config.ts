import type { Config } from 'tailwindcss'

const config: Config = {
  content: [
    './index.html',
    './src/**/*.{js,ts,jsx,tsx}',
    '../raisin-flow-designer/src/**/*.{ts,tsx,js,jsx}',
  ],
}

export default config
