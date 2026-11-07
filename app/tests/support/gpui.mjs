// gpui-kit and gpui-base for tests: every builder call returns the element, and `.child` keeps
// what it is given, so a test can read the text a view drew.
export class View {}
function element() {
  const texts = [];
  const self = new Proxy({}, {
    get(_, name) {
      if (name === 'texts') return texts;
      if (name === 'child') {
        return child => {
          texts.push(...(typeof child === 'string' ? [child] : child.texts));
          return self;
        };
      }
      if (name === 'children') {
        return children => {
          for (const child of children) self.child(child);
          return self;
        };
      }
      if (name === 'when') return (condition, then) => (condition ? then(self) : self);
      return () => self;
    },
  });
  return self;
}
export const div = element;
export const v_flex = element;
export const h_flex = element;
