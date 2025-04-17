/** @type {import('tailwindcss').Config} */
module.exports = {
    content: [
      "./*.html",
      "./src/**/*.{js,ts,jsx,tsx}"
    ],
    theme: {
      extend: {
        colors: {
          'alien-blue': '#0088CC',
          'alien-gray': '#1E1E1E',
          'alien-green': '#4CAF50',
          'red-700': '#B91C1C',
          'blue-700': '#1D4ED8',
        },
        spacing: {
          '6': '1.5rem',
          '4': '1rem',
        },
        fontSize: {
          '2xl': '1.5rem',
          'xl': '1.25rem',
          'sm': '0.875rem',
        },
        borderRadius: {
          'lg': '0.5rem',
          'full': '9999px',
        },
        animation: {
          'slow-pulse': 'pulse 3s linear infinite',
        }
      },
    },
    plugins: [],
  } 