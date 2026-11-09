import js from "@eslint/js";
import babelParser from "@babel/eslint-parser";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";

// TypeScript 7 is outside typescript-eslint 8's supported peer range.
// Parse its syntax without changing the compiler; tsc remains the type checker.
export default [
  {
    ignores: [
      "dist/**",
      "release/**",
      "release-output/**",
      ".fluxdl-release-work/**",
      "artifacts/**",
      "node_modules/**",
      "src-tauri/**",
      "licenses/**",
      "wiki_memory/**",
    ],
  },
  {
    files: ["src/**/*.{ts,tsx}", "vite.config.ts"],
    languageOptions: {
      parser: babelParser,
      parserOptions: {
        requireConfigFile: false,
        babelOptions: {
          babelrc: false,
          configFile: false,
          parserOpts: { plugins: ["typescript", "jsx"] },
        },
      },
      globals: { ...globals.browser, ...globals.node },
    },
    plugins: { "react-hooks": reactHooks },
    rules: {
      ...js.configs.recommended.rules,
      "no-undef": "off",
      "no-unused-vars": "off",
      "no-restricted-syntax": [
        "error",
        {
          selector: "TSAnyKeyword",
          message: "Use an explicit boundary type or unknown instead of any.",
        },
      ],
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "warn",
    },
  },
  {
    files: ["scripts/**/*.mjs", "eslint.config.mjs"],
    languageOptions: { globals: { ...globals.node, ...globals.browser } },
    rules: { ...js.configs.recommended.rules },
  },
];
