# Build Resolution Plan

The current build is failing because SvelteKit's Vite build process is attempting to generate server-side artifacts that are not needed for this static Tauri application, leading to missing file errors.

### Proposed Changes

1.  **Remove Hacky Workarounds**:
    *   Revert `package.json` changes (remove the `prebuild` script and associated file generation).
    *   Delete the `scripts/prebuild.cjs` file.
2.  **Properly Configure SvelteKit**:
    *   Ensure `svelte.config.js` is correctly configured for a purely static SPA application.
    *   Verify `src/routes/+page.ts` is configured for static prerendering.
3.  **Clean & Rebuild**:
    *   Completely remove the stale `.svelte-kit` and `build` output directories.
    *   Perform a clean sync and build to regenerate the necessary files based on the static configuration.

This approach resolves the build errors by correctly aligning the configuration with the intended output, rather than attempting to bypass the checks with file stubs.
