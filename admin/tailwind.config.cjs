module.exports = {
  content: ["./index.html", "./src/**/*.{ts,tsx,mjs}"],
  theme: {
    extend: {
      colors: {
        border: "hsl(var(--border))", input: "hsl(var(--border))", ring: "hsl(var(--primary))",
        background: "hsl(var(--background))", foreground: "hsl(var(--foreground))",
        primary: { DEFAULT: "hsl(var(--primary))", foreground: "#fff" },
        secondary: { DEFAULT: "#edf1ed", foreground: "#284636" },
        destructive: { DEFAULT: "#b33333", foreground: "#fff" },
        muted: { DEFAULT: "#f1f3ef", foreground: "#5f6a63" },
        accent: { DEFAULT: "#edf1ed", foreground: "#284636" }
      },
      fontFamily: { sans: ["-apple-system", "BlinkMacSystemFont", "Segoe UI", "sans-serif"] }
    }
  },
  plugins: [require("tailwindcss-animate")]
};
