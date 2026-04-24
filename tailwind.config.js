/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      colors: {
        'nm-bg': 'var(--nm-bg)',
        'nm-surface': 'var(--nm-surface)',
        'nm-text': 'var(--nm-text)',
        'nm-text-secondary': 'var(--nm-text-secondary)',
        'nm-accent': 'var(--nm-accent)',
      },
    },
  },
  plugins: [],
}
