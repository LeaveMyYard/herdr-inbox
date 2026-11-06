// Resolves the host's modules to stand-ins, so that a mod's main.js loads under Node.
export async function resolve(specifier, context, nextResolve) {
  if (specifier === 'herdr') return { url: new URL('./herdr.mjs', import.meta.url).href, shortCircuit: true };
  if (specifier === 'gpui-kit' || specifier === 'gpui-base') {
    return { url: new URL('./gpui.mjs', import.meta.url).href, shortCircuit: true };
  }
  return nextResolve(specifier, context);
}
