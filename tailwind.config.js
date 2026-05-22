/** @type {import('tailwindcss').Config} */

// Stomatopod marketing design tokens.
//
// The scale is built on the golden ratio (φ ≈ 1.618). Type, spacing,
// and container widths line up so adjacent elements relate by φ or its
// Fibonacci approximations (8, 13, 21, 34, 55, 89, 144). Shadows are
// multi-layered and slightly tinted toward the indigo accent so cards
// feel like they sit on the page instead of being stamped onto it.
module.exports = {
  content: ['./crates/web/templates/marketing/**/*.html'],
  theme: {
    extend: {
      colors: {
        ink: {
          DEFAULT: '#0b0d14',
          50:  '#f4f5f9',
          100: '#e6e7ee',
          200: '#c9cbd6',
          300: '#a3a7b8',
          400: '#7a7f93',
          500: '#5b6478',
          600: '#414858',
          700: '#2c3140',
          800: '#1a1d27',
          900: '#10131c',
          950: '#0b0d14',
        },
        surface: {
          DEFAULT: '#fbfbfd',
          warm:    '#f7f8fb',
          cool:    '#f4f5fa',
        },
        accent: {
          DEFAULT: '#6366f1',
          dark:    '#4f46e5',
          darker:  '#4338ca',
          light:   '#a5b4fc',
          softer:  '#c7d2fe',
        },
        muted: '#5b6478',
        gold:    '#eab308',
        teal:    '#06b6d4',
        magenta: '#d946ef',
      },
      fontFamily: {
        // Variable Inter if installed locally, otherwise refined system stack.
        // Kept self-hosted: no external font fetches, in line with the "no
        // third-party trackers" promise.
        sans: [
          'InterVariable', 'Inter',
          'ui-sans-serif', 'system-ui',
          '-apple-system', 'BlinkMacSystemFont',
          '"Segoe UI Variable Text"', '"Segoe UI"',
          'Helvetica', 'Arial', 'sans-serif',
        ],
        display: [
          'InterVariable', 'Inter',
          'ui-sans-serif', 'system-ui',
          '-apple-system', 'BlinkMacSystemFont',
          '"Segoe UI Variable Display"', '"Segoe UI"',
          'Helvetica', 'Arial', 'sans-serif',
        ],
        mono: [
          'ui-monospace', 'SFMono-Regular',
          '"JetBrains Mono"', '"Cascadia Code"',
          'Menlo', 'Monaco', 'Consolas', 'monospace',
        ],
      },
      fontSize: {
        // φ-aligned type scale. Display sizes get tighter tracking.
        'xs':   ['0.75rem',  { lineHeight: '1rem' }],
        'sm':   ['0.875rem', { lineHeight: '1.45' }],
        'base': ['0.9375rem',{ lineHeight: '1.618' }],
        'lg':   ['1.125rem', { lineHeight: '1.55' }],
        'xl':   ['1.4375rem',{ lineHeight: '1.45' }],
        '2xl':  ['1.875rem', { lineHeight: '1.20', letterSpacing: '-0.011em' }],
        '3xl':  ['2.375rem', { lineHeight: '1.10', letterSpacing: '-0.016em' }],
        '4xl':  ['3rem',     { lineHeight: '1.04', letterSpacing: '-0.020em' }],
        '5xl':  ['3.75rem',  { lineHeight: '1.02', letterSpacing: '-0.024em' }],
        '6xl':  ['4.875rem', { lineHeight: '0.98', letterSpacing: '-0.028em' }],
        '7xl':  ['5.5rem',   { lineHeight: '0.96', letterSpacing: '-0.030em' }],
      },
      letterSpacing: {
        tightest: '-0.04em',
        snug: '-0.012em',
        eyebrow: '0.18em',
      },
      spacing: {
        // Fibonacci-ish supplementary scale.
        'phi-1': '0.5rem',     // 8
        'phi-2': '0.8125rem',  // 13
        'phi-3': '1.3125rem',  // 21
        'phi-4': '2.125rem',   // 34
        'phi-5': '3.4375rem',  // 55
        'phi-6': '5.5625rem',  // 89
        'phi-7': '9rem',       // 144
      },
      maxWidth: {
        prose: '70ch',
        golden: '38.2rem',
        'golden-lg': '61.8rem',
      },
      backgroundImage: {
        // Multi-stop radial mesh — a still painting under the WebGL canvas
        // so the hero is beautiful even without JS or WebGL.
        'hero-gradient': "radial-gradient(60% 80% at 12% 8%, rgba(99,102,241,0.55) 0%, transparent 62%), radial-gradient(55% 75% at 88% 22%, rgba(6,182,212,0.28) 0%, transparent 60%), radial-gradient(50% 70% at 60% 110%, rgba(217,70,239,0.30) 0%, transparent 65%), linear-gradient(168deg, #0b0d14 0%, #14162a 48%, #1e1b4b 100%)",
        'soft-mesh': "radial-gradient(900px 500px at 0% -10%, rgba(99,102,241,0.07), transparent 60%), radial-gradient(700px 500px at 100% 110%, rgba(217,70,239,0.05), transparent 60%)",
        'glow-radial': 'radial-gradient(circle at 30% 20%, rgba(99,102,241,0.42), transparent 60%)',
        'noise': "url(\"data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 220 220'><filter id='n'><feTurbulence type='fractalNoise' baseFrequency='0.9' numOctaves='2' stitchTiles='stitch'/><feColorMatrix values='0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0 0 0 0.55 0'/></filter><rect width='220' height='220' filter='url(%23n)'/></svg>\")",
        'shimmer': 'linear-gradient(90deg, transparent 0%, rgba(255,255,255,0.5) 50%, transparent 100%)',
      },
      boxShadow: {
        // Multi-layered, slightly indigo-tinted. The lowest layer is a
        // crisp 1px to anchor the edge; the upper layers do the diffusion.
        'xs':            '0 1px 0 rgba(15,17,23,0.04)',
        'card':          '0 1px 0 rgba(15,17,23,0.04), 0 2px 6px rgba(15,17,23,0.04), 0 10px 22px rgba(15,17,23,0.05), 0 22px 44px rgba(15,17,23,0.04)',
        'card-hover':    '0 1px 0 rgba(15,17,23,0.04), 0 4px 10px rgba(15,17,23,0.05), 0 16px 36px rgba(15,17,23,0.07), 0 36px 72px rgba(99,102,241,0.14)',
        'pop':           '0 1px 0 rgba(0,0,0,0.06), 0 6px 14px rgba(99,102,241,0.28), 0 20px 40px rgba(99,102,241,0.18)',
        'pop-hover':     '0 1px 0 rgba(0,0,0,0.06), 0 10px 22px rgba(99,102,241,0.36), 0 28px 56px rgba(99,102,241,0.24)',
        'inner-hi':      'inset 0 1px 0 rgba(255,255,255,0.6)',
        'inner-hi-dark': 'inset 0 1px 0 rgba(255,255,255,0.10)',
      },
      borderRadius: {
        'xl':  '0.875rem',
        '2xl': '1.125rem',
        '3xl': '1.625rem',
        '4xl': '2.125rem',
      },
      animation: {
        'shimmer':    'shimmer 7s linear infinite',
        'float-slow': 'floatSlow 14s ease-in-out infinite',
        'pulse-glow': 'pulseGlow 3.2s ease-in-out infinite',
      },
      keyframes: {
        shimmer: {
          '0%':   { backgroundPosition: '-200% 0' },
          '100%': { backgroundPosition: '200% 0' },
        },
        floatSlow: {
          '0%, 100%': { transform: 'translateY(0) translateX(0)' },
          '50%':      { transform: 'translateY(-10px) translateX(4px)' },
        },
        pulseGlow: {
          '0%, 100%': { boxShadow: '0 0 0 0 rgba(99,102,241,0.45)' },
          '50%':      { boxShadow: '0 0 0 10px rgba(99,102,241,0)' },
        },
      },
    },
  },
  plugins: [require('@tailwindcss/forms'), require('@tailwindcss/typography')],
};
