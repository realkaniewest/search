// Focused regression checks for the static UI prototype. No browser packages needed.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

class Element {
  constructor(tagName = 'div') {
    this.tagName = tagName;
    this.children = [];
    this.attributes = new Map();
    this.listeners = new Map();
    this.dataset = {};
    this.style = {};
    this.value = '';
    this.disabled = false;
  }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = [...children]; }
  setAttribute(name, value) { this.attributes.set(name, String(value)); }
  getAttribute(name) { return this.attributes.get(name) ?? null; }
  addEventListener(name, callback) { this.listeners.set(name, callback); }
  querySelector(selector) {
    if (selector === '[aria-selected="true"]') {
      return this.children.flatMap(child => [child, ...child.children]).find(child => child.getAttribute('aria-selected') === 'true') ?? null;
    }
    throw new Error(`Unexpected selector: ${selector}`);
  }
  scrollIntoView() {}
  focus() { document.activeElement = this; }
  dispatch(name, extra = {}) {
    const handler = this.listeners.get(name);
    assert.ok(handler, `${this.tagName} must handle ${name}`);
    handler({ target: this, key: extra.key, metaKey: false, ctrlKey: false, preventDefault() {}, stopPropagation() {}, ...extra });
  }
  pressActivationKey(key) {
    assert.ok(key === 'Enter' || key === ' ', 'only native button activation keys are modeled');
    assert.equal(this.tagName, 'button', 'tab must use a native keyboard-activatable button');
    this.focus();
    this.dispatch('click'); // Browser dispatches click for Enter/Space on a button.
  }
}

const elements = new Map(['tabs', 'content', 'address', 'back', 'forward', 'new-tab', 'theme'].map(id => [id, new Element()]));
const document = {
  activeElement: null,
  documentElement: { dataset: {} },
  createElement: tag => new Element(tag),
  getElementById: id => elements.get(id),
  addEventListener() {},
  title: ''
};
const html = readFileSync(new URL('./index.html', import.meta.url), 'utf8');
const script = html.match(/<script>([\s\S]*?)<\/script>/)?.[1];
assert.ok(script, 'inline prototype script exists');
runInNewContext(script, { document, Object, String, Number });

const list = elements.get('tabs');
const panel = elements.get('content');
const selected = () => list.querySelector('[aria-selected="true"]');
const findClass = (root, className) => root.className === className
  ? root
  : root.children.map(child => findClass(child, className)).find(Boolean);
assert.equal(list.children.length, 1);
assert.equal(selected().getAttribute('aria-controls'), 'content');
assert.equal(panel.getAttribute('aria-labelledby'), selected().id);

elements.get('new-tab').dispatch('click');
assert.equal(list.children.length, 2);
let slot = list.children[0];
assert.equal(slot.children.length, 2, 'tab and close control are siblings');
assert.equal(slot.children[0].getAttribute('role'), 'tab');
assert.equal(slot.children[1].tagName, 'button');

slot.children[0].dispatch('click');
assert.equal(document.activeElement, selected(), 'click selection retains keyboard focus');
assert.equal(panel.getAttribute('aria-labelledby'), selected().id);

const secondId = list.children[1].children[0].id;
list.children[1].children[0].pressActivationKey('Enter');
assert.equal(selected().id, secondId, 'Enter activates the second tab');
assert.equal(document.activeElement, selected(), 'Enter activation retains keyboard focus');

const firstId = list.children[0].children[0].id;
list.children[0].children[0].pressActivationKey(' ');
assert.equal(selected().id, firstId, 'Space activates the first tab');
assert.equal(document.activeElement, selected(), 'Space activation retains keyboard focus');

list.dispatch('keydown', { target: selected(), key: 'ArrowRight' });
assert.equal(document.activeElement, selected(), 'arrow selection retains keyboard focus');

slot = list.children[0];
slot.children[1].dispatch('click');
assert.equal(list.children.length, 1);
assert.equal(document.activeElement, selected(), 'closing a tab focuses the remaining tab');

findClass(panel, 'favorite').dispatch('click');
assert.equal(list.children.length, 2);
assert.equal(document.activeElement, selected(), 'opening a favorite focuses its new tab');

elements.get('new-tab').dispatch('click');
findClass(panel, 'recent-link').dispatch('click');
assert.equal(document.activeElement, selected(), 'opening a recent site focuses its new tab');

assert.ok(!html.includes('.actions .tool:last-child { display: none; }'), 'theme control remains available at narrow widths');

const theme = elements.get('theme');
theme.dispatch('click');
assert.equal(theme.getAttribute('aria-pressed'), 'true');
theme.dispatch('click');
assert.equal(theme.getAttribute('aria-pressed'), 'false');
console.log('UI shell focus, tab semantics and theme checks passed');
