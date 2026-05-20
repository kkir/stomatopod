/** @type {import('tailwindcss').Config} */
module.exports = {
  content: ['./crates/web/templates/marketing/**/*.html'],
  theme: {
    extend: {
      colors: {
        ink: '#0f1117',
        surface: '#f7f8fb',
        accent: {
          DEFAULT: '#6366f1',
          dark: '#4f46e5',
          light: '#a5b4fc',
        },
        muted: '#5b6478',
      },
      fontFamily: {
        sans: ['system-ui', '-apple-system', 'Segoe UI', 'Helvetica', 'Arial', 'sans-serif'],
        mono: ['ui-monospace', 'SFMono-Regular', 'Menlo', 'Monaco', 'monospace'],
      },
      backgroundImage: {
        'hero-gradient': 'linear-gradient(135deg, #0f1117 0%, #1e1b4b 55%, #312e81 100%)',
        'glow-radial': 'radial-gradient(circle at 30% 20%, rgba(99,102,241,0.35), transparent 60%)',
      },
      boxShadow: {
        'card': '0 1px 2px rgba(15,17,23,0.04), 0 8px 24px rgba(15,17,23,0.06)',
        'card-hover': '0 4px 12px rgba(15,17,23,0.08), 0 16px 40px rgba(99,102,241,0.12)',
      },
      maxWidth: {
        prose: '70ch',
      },
    },
  },
  plugins: [require('@tailwindcss/forms'), require('@tailwindcss/typography')],
};
