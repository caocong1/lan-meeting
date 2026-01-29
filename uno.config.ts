import { defineConfig, presetUno, presetIcons } from "unocss";

export default defineConfig({
  presets: [
    presetUno(),
    presetIcons({
      scale: 1.2,
      extraProperties: {
        'display': 'inline-block',
        'vertical-align': 'middle',
      },
      collections: {
        lucide: () => import('@iconify-json/lucide/icons.json').then(i => i.default),
      },
    }),
  ],
  theme: {
    colors: {
      primary: {
        50: "#eff6ff",
        100: "#dbeafe",
        200: "#bfdbfe",
        300: "#93c5fd",
        400: "#60a5fa",
        500: "#3b82f6",
        600: "#2563eb",
        700: "#1d4ed8",
        800: "#1e40af",
        900: "#1e3a8a",
      },
    },
  },
  shortcuts: {
    btn: "px-4 py-2 rounded-lg font-medium transition-colors cursor-pointer",
    "btn-primary": "btn bg-primary-500 text-white hover:bg-primary-600",
    "btn-secondary": "btn bg-gray-200 text-gray-800 hover:bg-gray-300",
    card: "bg-white rounded-xl shadow-sm border border-gray-200 p-4",
  },
});
