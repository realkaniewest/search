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
    this.clientWidth = this.scrollWidth = 500;
    this.offsetWidth = 24;
    this.scrollLeft = 0;
    this.hidden = true;
  }
  set innerHTML(_value) { assert.fail('chrome must build DOM nodes without HTML injection'); }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = [...children]; }
  setAttribute(key, value) { this.attributes.set(key, String(value)); }
  getAttribute(key) { return this.attributes.get(key); }
  addEventListener(key, callback) { this.listeners.set(key, callback); }
  focus() { document.activeElement = this; }
  blur() { document.activeElement = null; }
  select() {}
  scrollBy({left}) {
    this.scrollLeft = Math.max(0, Math.min(this.scrollLeft + left, this.scrollWidth - this.clientWidth));
    this.dispatch('scroll');
  }
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

const elements = new Map(['tabs', 'address', 'notice', 'activity', 'add', 'back', 'forward', 'reload', 'tabs-left', 'tabs-right'].map(id => [id, new Element()]));
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
let resizeTabs;
class ResizeObserver {
  constructor(callback) { resizeTabs = callback; }
  observe() {}
}
runInNewContext(script, { document, window, Number, JSON, TextEncoder, ResizeObserver });

const tabs = elements.get('tabs');
window.renderSearch({active: 0, tabs: [
  {id: 10, label: 'Новая вкладка', url: 'search:start'},
  {id: 11, label: 'Rust', url: 'https://rust-lang.org/'},
]});
assert.equal(tabs.children.length, 2);
assert.equal(elements.get('back').disabled, true, 'new tab has no back entry');
assert.equal(elements.get('forward').disabled, true, 'new tab has no forward entry');
window.renderSearch({...window.searchState, can_back: true, can_forward: true});
assert.equal(elements.get('back').disabled, false);
assert.equal(elements.get('forward').disabled, false);
assert.equal(elements.get('reload').disabled, true, 'start page has no reload history entry');
assert.equal(tabs.children[0].children[0].getAttribute('role'), 'tab');
assert.equal(tabs.children[0].children[1].tagName, 'button', 'close is a separate keyboard button');
assert.equal(tabs.children[0].children[0].tabIndex, 0);
assert.equal(tabs.children[1].children[0].tabIndex, -1);
assert.equal(elements.get('activity').textContent, '');

window.renderSearch({active: 1, tabs: [
  {id: 10, label: 'Новая вкладка', url: 'search:start', loading: false},
  {id: 11, label: 'Rust', url: 'https://rust-lang.org/', loading: true},
]});
assert.equal(elements.get('activity').textContent, 'Загрузка страницы…');
assert.equal(elements.get('reload').disabled, false);
assert.equal(tabs.children[1].children[0].getAttribute('aria-label'), 'Rust загружается');
assert.equal(tabs.children[1].children[0].children[0].className, 'dot loading');
assert.equal(tabs.children[1].children[0].scrolledWith.inline, 'nearest', 'active tab stays visible');
window.renderSearch({active: 1, tabs: [
  {id: 10, label: 'Новая вкладка', url: 'search:start', loading: false},
  {id: 11, label: 'Rust Programming Language', url: 'https://rust-lang.org/', loading: false},
]});
assert.equal(tabs.children[1].children[0].scrolledWith, undefined, 'title updates do not reset manual scrolling');
const left = elements.get('tabs-left');
const right = elements.get('tabs-right');
tabs.scrollWidth = 900;
resizeTabs();
assert.equal(left.hidden, false);
assert.equal(left.disabled, true);
assert.equal(right.disabled, false);
right.dispatch('click');
assert.equal(right.disabled, true);
left.dispatch('click');
assert.equal(left.disabled, true);
tabs.scrollWidth = tabs.clientWidth;
resizeTabs();
assert.equal(left.hidden, true);
assert.equal(right.hidden, true);
window.renderSearch({active: 0, tabs: [
  {id: 10, label: 'Новая вкладка', url: 'search:start', loading: false},
  {id: 11, label: 'Rust', url: 'https://rust-lang.org/', loading: false},
]});
assert.equal(elements.get('activity').textContent, '');

const first = tabs.children[0].children[0];
first.focus();
tabs.dispatch('keydown', {target: first, key: 'ArrowRight'});
assert.deepEqual(messages.pop(), {type: 'select', id: 11});
assert.equal(document.activeElement.dataset.index, 1);
window.renderSearch({active: 1, tabs: [
  {id: 10, label: 'Новая вкладка', url: 'search:start'},
  {id: 11, label: 'Rust', url: 'https://rust-lang.org/'},
]});
assert.equal(document.activeElement.dataset.index, 1, 'rerender keeps tab focus');
tabs.dispatch('keydown', {target: document.activeElement, key: 'Home'});
assert.deepEqual(messages.pop(), {type: 'select', id: 10});
tabs.dispatch('keydown', {target: document.activeElement, key: 'End'});
assert.deepEqual(messages.pop(), {type: 'select', id: 11});
tabs.dispatch('keydown', {target: document.activeElement, key: 'ArrowLeft'});
assert.deepEqual(messages.pop(), {type: 'select', id: 10});
tabs.dispatch('keydown', {target: document.activeElement, key: 'Delete'});
assert.deepEqual(messages.pop(), {type: 'close', id: 10});
const closeRequest = tabs.children[1].children[1];
closeRequest.dispatch('click');
closeRequest.dispatch('click');
assert.deepEqual(messages.splice(-2), [{type: 'close', id: 11}, {type: 'close', id: 11}]);

elements.get('reload').dispatch('click');
assert.deepEqual(messages.pop(), {type: 'reload'});

const address = elements.get('address');
address.focus();
address.value = 'https://www.wikipedia.org/wiki/Rust';
window.renderSearch({active: 1, tabs: [
  {id: 10, label: 'Новая вкладка', url: 'search:start'},
  {id: 11, label: 'Rust', url: 'https://rust-lang.org/learn'},
]});
assert.equal(address.value, 'https://www.wikipedia.org/wiki/Rust', 'page update must not erase typed URL');
window.renderSearch({active: 0, tabs: [
  {id: 10, label: 'Новая вкладка', url: 'search:start'},
  {id: 11, label: 'Rust', url: 'https://rust-lang.org/learn'},
]});
assert.equal(address.value, '', 'switching tabs must replace the old typed URL');
address.blur();
window.renderSearch({active: 1, tabs: [
  {id: 10, label: 'Новая вкладка', url: 'search:start'},
  {id: 11, label: 'Rust', url: 'https://rust-lang.org/learn'},
]});
assert.equal(address.value, 'https://rust-lang.org/learn');

window.renderSearch({active: 1, tabs: [
  {id: 10, label: 'Новая вкладка', url: 'search:start'},
  {id: 11, label: 'Rust', url: 'https://rust-lang.org/learn'},
], notice: 'Пока доступны только выбранные сайты.', attempted_address: 'https://blocked.invalid/'});
assert.equal(address.value, 'https://blocked.invalid/', 'rejected address stays editable');
assert.equal(address.getAttribute('aria-invalid'), 'true');
assert.equal(elements.get('notice').textContent, 'Пока доступны только выбранные сайты.');
address.focus();
address.dispatch('keydown', {key: 'Escape', isComposing: true});
assert.equal(address.value, 'https://blocked.invalid/', 'composition owns Escape');
address.dispatch('keydown', {key: 'Escape'});
assert.deepEqual(messages.pop(), {type: 'cancel-address'});
assert.equal(address.value, 'https://rust-lang.org/learn', 'Escape restores the current URL');
assert.equal(address.getAttribute('aria-invalid'), 'false');
assert.equal(elements.get('notice').textContent, '');
address.blur();
window.renderSearch({active: 1, tabs: [
  {id: 10, label: 'Новая вкладка', url: 'search:start'},
  {id: 11, label: 'Rust', url: 'https://rust-lang.org/learn'},
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

const stableTab = tabs.children[1].children[0];
stableTab.focus();
window.renderSearch({
  ...window.searchState,
  notice: 'Открыто 12 вкладок.',
});
assert.equal(tabs.children[1].children[0], stableTab, 'notice does not recreate tab buttons');
assert.equal(document.activeElement, stableTab, 'notice preserves keyboard focus');

const untrustedLabel = '<img src=x> & test';
window.renderSearch({
  ...window.searchState,
  tabs: window.searchState.tabs.map((tab, i) => i === 1 ? {...tab, label: untrustedLabel} : tab),
});
const titleNode = tabs.children[1].children[0].children[1];
assert.equal(titleNode.textContent, untrustedLabel, 'site title stays text');
assert.equal(titleNode.children.length, 0, 'site title does not create HTML nodes');

address.focus();
address.value = 'typing in previous tab';
window.renderSearch({
  ...window.searchState,
  notice: null,
  tabs: window.searchState.tabs.map((tab, i) => i === window.searchState.active
    ? {...tab, id: 42, url: 'https://rust-lang.org/learn'} : tab),
});
assert.equal(address.value, 'https://rust-lang.org/learn', 'replacement tab clears previous address draft');

const firstId = {id: 10, label: 'Начало', url: 'search:start'};
const secondId = {id: 11, label: 'Rust', url: 'https://rust-lang.org/'};
const thirdId = {id: 12, label: 'Wikipedia', url: 'https://www.wikipedia.org/'};
window.renderSearch({active: 1, tabs: [firstId, secondId, thirdId]});
tabs.children[1].children[1].focus();
window.renderSearch({active: 0, tabs: [secondId, thirdId]});
assert.equal(document.activeElement, tabs.children[0].children[1], 'focus follows the surviving tab ID');
window.renderSearch({active: 0, tabs: [thirdId]});
assert.equal(document.activeElement, tabs.children[0].children[0], 'removed close button yields focus to selected tab');

window.renderSearch({active: 1, tabs: [firstId, secondId, thirdId]});
address.focus();
address.value = 'editing current tab';
window.renderSearch({active: 0, tabs: [secondId, thirdId]});
assert.equal(address.value, 'editing current tab', 'closing a preceding tab preserves the active draft');

console.log('Search chrome keyboard and tab controls passed');
