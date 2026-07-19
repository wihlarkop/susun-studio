import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

export default defineConfig({
  resolve: {
    alias: {
      // SvelteKit's own `$lib` default, so test files can import the same
      // way production code does instead of falling back to relative
      // paths. `vitest.config.ts` is separate from `vite.config.js` and
      // does not load the `sveltekit()` plugin (which sets this up for the
      // app itself), so it needs restating here.
      $lib: fileURLToPath(new URL("./src/lib", import.meta.url)),
    },
  },
  test: {
    passWithNoTests: true,
  },
});
