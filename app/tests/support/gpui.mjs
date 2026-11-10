// gpui-kit and gpui-base for tests: every builder call returns the element, and `.child` keeps
// what it is given, so a test can read the text a view drew. An element with an `id` is kept in
// `ids` with its click handler, the styles it was given and its text, so a test can click what was drawn.
export class View {}
function element() {
  const texts = [];
  const ids = new Map();
  const own = { id: null, click: null, calls: [], texts };
  const self = new Proxy({}, {
    get(_, name) {
      if (name === 'texts') return texts;
      if (name === 'ids') return ids;
      if (name === 'child') {
        return child => {
          texts.push(...(typeof child === 'string' ? [child] : child.texts));
          if (typeof child !== 'string') for (const [id, el] of child.ids) ids.set(id, el);
          return self;
        };
      }
      if (name === 'children') {
        return children => {
          for (const child of children) self.child(child);
          return self;
        };
      }
      if (name === 'map') return transform => transform(self);
      if (name === 'when') return (condition, then) => (condition ? then(self) : self);
      if (name === 'id') {
        return id => {
          own.id = id;
          ids.set(id, own);
          return self;
        };
      }
      if (name === 'on_click') {
        return handler => {
          own.click = handler;
          return self;
        };
      }
      return (...args) => {
        own.calls.push([name, ...args]);
        return self;
      };
    },
  });
  return self;
}
export const div = element;
export const v_flex = element;
export const h_flex = element;
