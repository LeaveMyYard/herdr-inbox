// The `herdr` module for tests: a test sets `world` to what the host should answer.
export const world = { home: '/home/fake', appHome: '/home/fake/app', files: {}, exec: async () => ({ code: 1, stdout: '', stderr: '' }) };
export const home = () => world.home;
export const appHome = () => world.appHome;
export const readTextFile = async path => world.files[path] ?? null;
export const exec = (command, args) => world.exec(command, args);
export const theme = () => new Proxy({}, { get: (_, name) => String(name) });
export const agents = () => world.agents ?? [];
export const closeOverlay = id => { (world.closed ??= []).push(id); return null; };
export const notify = n => { (world.toasts ??= []).push(n); return 'n'; };
export const props = () => world.props ?? {};
export const run = (name, args) => { (world.ran ??= []).push([name, args]); return null; };
export const watchFolder = () => new Promise(() => {});
export const settings = () => ({});
