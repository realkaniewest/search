import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

class Element {
  constructor(tagName = 'div') {
    this.tagName = tagName;
    this.children = [];
    this.dataset = {};
    this.attributes = new Map();
    this.listeners = new Map();
    this.value = '';
  }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = [...children]; }
  setAttribute(key, value) { this.attributes.set(key, String(value)); }
  getAttribute(key) { return this.attributes.get(key); }
  addEventListener(key, callback) { this.listeners.set(key, callback); }
  focus() { document.activeElement = this; }
  blur() { document.activeElement = null; }
  select() {}
  scrollIntoView(options) { this.scrolledWith = options; }
  closest(selector) { return selector === '[role="tab"]' && this.getAttribute('role') === 'tab' ? this : null; }
  querySelector(selector) {
    const action = selector.match(/data-action="([^"]+)"/)?.[1];
    const index = selector.match(/data-index="([^"]+)"/)?.[1];
    const role = selector.includes('[role="tab"]') ? 'tab' : undefined;
    const visit = element => {
      if ((action === undefined || element.dataset.action === action)
          && (index === undefined || String(element.dataset.index) === index)
          && (role === undefined || element.getAttribute('role') === role)) return element;
      for (const child of element.children) {
        const found = visit(child);
        if (found) return found;
      }
      return null;
    };
    return this.children.map(visit).find(Boolean) ?? null;
  }
  dispatch(key, extra = {}) {
    const callback = this.listeners.get(key) ?? this[`on${key}`];
    assert.ok(callback, `missing ${key} handler`);
    callback({ target: this, key: extra.key, metaKey: false, ctrlKey: false,
      preventDefault() {}, ...extra });
  }
}

const elements = new Map(['tabs', 'address', 'notice', 'activity', 'add', 'back', 'forward', 'reload'].map(id => [id, new Element()]));
const document = {
  activeElement: null,
  getElementById: id => elements.get(id),
  createElement: tag => new Element(tag),
  addEventListener() {},
};
const messages = [];
const window = { ipc: { postMessage: message => messages.push(JSON.parse(message)) } };
const html = readFileSync(new URL('./src/chrome.html', import.meta.url), 'utf8');
const script = html.match(/<script>([\s\S]*?)<\/script>/)?.[1];
assert.ok(script, 'Search chrome script exists');
runInNewContext(script, { document, window, Number, JSON, TextEncoder });

const tabs = elements.get('tabs');
window.renderSearch({active: 0, tabs: [
  {label: 'Новая вкладка', url: 'search:start'},
  {label: 'Rust', url: 'https://rust-lang.org/'},
]});
assert.equal(tabs.children.length, 2);
assert.equal(elements.get('reload').disabled, true, 'start page has no reload history entry');
assert.equal(tabs.children[0].children[0].getAttribute('role'), 'tab');
assert.equal(tabs.children[0].children[1].tagName, 'button', 'close is a separate keyboard button');
assert.equal(tabs.children[0].children[0].tabIndex, 0);
assert.equal(tabs.children[1].children[0].tabIndex, -1);
assert.equal(elements.get('activity').textContent, '');

window.renderSearch({active: 1, tabs: [
  {label: 'Новая вкладка', url: 'search:start', loading: false},
  {label: 'Rust', url: 'https://rust-lang.org/', loading: true},
]});
assert.equal(elements.get('activity').textContent, 'Загрузка страницы…');
assert.equal(elements.get('reload').disabled, false);
assert.equal(tabs.children[1].children[0].getAttribute('aria-label'), 'Rust загружается');
assert.equal(tabs.children[1].children[0].children[0].className, 'dot loading');
assert.equal(tabs.children[1].children[0].scrolledWith.inline, 'nearest', 'active tab stays visible');
window.renderSearch({active: 1, tabs: [
  {label: 'Новая вкладка', url: 'search:start', loading: false},
  {label: 'Rust Programming Language', url: 'https://rust-lang.org/', loading: false},
]});
assert.equal(tabs.children[1].children[0].scrolledWith.inline, 'nearest', 'active tab stays visible after title update');
window.renderSearch({active: 0, tabs: [
  {label: 'Новая вкладка', url: 'search:start', loading: false},
  {label: 'Rust', url: 'https://rust-lang.org/', loading: false},
]});
assert.equal(elements.get('activity').textContent, '');

const first = tabs.children[0].children[0];
first.focus();
tabs.dispatch('keydown', {target: first, key: 'ArrowRight'});
assert.deepEqual(messages.pop(), {type: 'select', index: 1});
assert.equal(document.activeElement.dataset.index, 1);
window.renderSearch({active: 1, tabs: [
  {label: 'Новая вкладка', url: 'search:start'},
  {label: 'Rust', url: 'https://rust-lang.org/'},
]});
assert.equal(document.activeElement.dataset.index, 1, 'rerender keeps tab focus');
tabs.dispatch('keydown', {target: document.activeElement, key: 'Home'});
assert.deepEqual(messages.pop(), {type: 'select', index: 0});
tabs.dispatch('keydown', {target: document.activeElement, key: 'End'});
assert.deepEqual(messages.pop(), {type: 'select', index: 1});
tabs.dispatch('keydown', {target: document.activeElement, key: 'ArrowLeft'});
assert.deepEqual(messages.pop(), {type: 'select', index: 0});
tabs.dispatch('keydown', {target: document.activeElement, key: 'Delete'});
assert.deepEqual(messages.pop(), {type: 'close', index: 0});
tabs.children[1].children[1].dispatch('click');
assert.deepEqual(messages.pop(), {type: 'close', index: 1});

elements.get('reload').dispatch('click');
assert.deepEqual(messages.pop(), {type: 'reload'});

const address = elements.get('address');
address.focus();
address.value = 'https://www.wikipedia.org/wiki/Rust';
window.renderSearch({active: 1, tabs: [
  {label: 'Новая вкладка', url: 'search:start'},
  {label: 'Rust', url: 'https://rust-lang.org/learn'},
]});
assert.equal(address.value, 'https://www.wikipedia.org/wiki/Rust', 'page update must not erase typed URL');
window.renderSearch({active: 0, tabs: [
  {label: 'Новая вкладка', url: 'search:start'},
  {label: 'Rust', url: 'https://rust-lang.org/learn'},
]});
assert.equal(address.value, '', 'switching tabs must replace the old typed URL');
address.blur();
window.renderSearch({active: 1, tabs: [
  {label: 'Новая вкладка', url: 'search:start'},
  {label: 'Rust', url: 'https://rust-lang.org/learn'},
]});
assert.equal(address.value, 'https://rust-lang.org/learn');

window.renderSearch({active: 1, tabs: [
  {label: 'Новая вкладка', url: 'search:start'},
  {label: 'Rust', url: 'https://rust-lang.org/learn'},
], notice: 'Пока доступны только выбранные сайты.', attempted_address: 'https://blocked.invalid/'});
assert.equal(address.value, 'https://blocked.invalid/', 'rejected address stays editable');
assert.equal(address.getAttribute('aria-invalid'), 'true');
assert.equal(elements.get('notice').textContent, 'Пока доступны только выбранные сайты.');
window.renderSearch({active: 1, tabs: [
  {label: 'Новая вкладка', url: 'search:start'},
  {label: 'Rust', url: 'https://rust-lang.org/learn'},
], notice: 'Открыто 12 вкладок.'});
assert.equal(address.getAttribute('aria-invalid'), 'false', 'other notices do not mark the address invalid');
address.dispatch('input');
assert.equal(elements.get('notice').textContent, '');
assert.equal(address.getAttribute('aria-invalid'), 'false');
address.value = 'я'.repeat(600);
const messagesBeforeLongAddress = messages.length;
address.dispatch('keydown', {key: 'Enter'});
assert.equal(messages.length, messagesBeforeLongAddress, 'oversize UTF-8 address never reaches IPC');
assert.equal(address.getAttribute('aria-invalid'), 'true');
assert.ok(elements.get('notice').textContent.includes('слишком длинный'));
address.value = '\\'.repeat(1024);
address.dispatch('keydown', {key: 'Enter'});
assert.equal(messages.length, messagesBeforeLongAddress, 'escaped JSON over limit never reaches IPC');

console.log('Search chrome keyboard and tab controls passed');
