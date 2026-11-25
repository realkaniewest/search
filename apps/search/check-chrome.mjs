import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

class Element {
  constructor(tagName = 'div') {
    this.tagName = tagName;
    this.children = [];
    this.className = '';
    this.classList = {
      add: value => { this.className += ' ' + value; },
      contains: value => this.className.split(' ').includes(value),
      remove: value => { this.className = this.className.split(' ').filter(name => name !== value).join(' '); },
    };
    this.selectionStart = this.selectionEnd = 0;
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
  insertBefore(child) { this.children.unshift(child); }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = [...children]; }
  setAttribute(key, value) { this.attributes.set(key, String(value)); }
  toggleAttribute(key, enabled) {
    if (enabled) this.attributes.set(key, '');
    else this.attributes.delete(key);
  }
  getAttribute(key) { return this.attributes.get(key); }
  addEventListener(key, callback) { this.listeners.set(key, callback); }
  focus() { document.activeElement = this; }
  blur() { document.activeElement = null; this.listeners.get('blur')?.(); }
  setSelectionRange(start, end) { this.selectionStart = start; this.selectionEnd = end; }
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

const elements = new Map(['tabs', 'address', 'notice', 'activity', 'add', 'back', 'forward', 'reload', 'tabs-left', 'tabs-right', 'composer', 'composer-panel', 'engine-toggle', 'engine-menu', 'engine-mark', 'engine-google', 'engine-yandex', 'engine-duckduckgo', 'engine-bing', 'history', 'history-open', 'history-filter', 'history-list', 'history-clear', 'history-close', 'history-status', 'bookmarks-open', 'bookmark-toggle', 'library-title', 'library-error', 'private-window', 'download-dialog', 'download-open', 'download-address', 'download-close', 'download-save', 'download-form', 'developer-window', 'inspect-page', 'toolbar', 'settings', 'settings-open', 'settings-close', 'settings-filter', 'settings-empty', 'settings-form', 'settings-save', 'settings-reset', 'settings-status', 'choose-download-folder', 'download-custom-option', 'download-custom-path'].map(id => [id, new Element()]));
const document = {
  activeElement: null,
  documentElement: new Element(),
  getElementById: id => elements.get(id),
  createElement: tag => new Element(tag),
  addEventListener(type, callback) { this[type] = callback; },
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
const controls = new Map([
  ['engine', {value:'google'}], ['private_engine', {value:'google'}], ['palette', {value:'graphite'}], ['page_zoom', {value:'100'}], ['download_folder', {value:'recent'}],
  ...['compact_tabs', 'reduced_motion', 'opaque_toolbar', 'restore_tabs', 'record_history'].map(name => [name, {type:'checkbox'}]),
]);
elements.get('settings-form').elements = { namedItem: name => controls.get(name) };
elements.get('settings-form').querySelectorAll = () => [];
runInNewContext(script, { document, window, Number, JSON, TextEncoder, ResizeObserver,
  matchMedia: () => ({matches: false}) });
const savedSettings = {engine:'google', private_engine:'duckduckgo', palette:'system', compact_tabs:true,
  reduced_motion:false, opaque_toolbar:false, restore_tabs:true, record_history:true, page_zoom:'100', download_folder:'recent'};
window.renderSettings(savedSettings, true, null);
assert.equal(document.documentElement.dataset.palette, 'paper');
assert.equal(document.documentElement.getAttribute('data-compact'), '');
assert.equal(elements.get('engine-mark').dataset.engine, 'duckduckgo');
window.renderSettings(savedSettings, false, null);
elements.get('settings-open').dispatch('click');
assert.deepEqual(messages.pop(), {type:'composer', visible:true});
assert.equal(elements.get('toolbar').inert, true);
controls.get('record_history').checked = false;
elements.get('settings-form').dispatch('change');
elements.get('settings-form').dispatch('submit');
assert.deepEqual(messages.pop(), {type:'save-settings', settings:{...savedSettings, record_history:false}});
window.renderSettings(savedSettings, false, 'failed');
assert.equal(elements.get('settings-save').disabled, false);
elements.get('settings-form').dispatch('submit');
messages.pop();
window.renderSettings({...savedSettings, record_history:false}, false, 'saved');
assert.equal(elements.get('settings-save').disabled, true);
elements.get('choose-download-folder').dispatch('click');
assert.deepEqual(messages.pop(), {type:'choose-download-folder'});
assert.equal(elements.get('choose-download-folder').disabled, true);
window.renderDownloadFolder('/tmp/Search downloads', 'selected');
assert.equal(elements.get('download-custom-path').textContent, '/tmp/Search downloads');
assert.equal(elements.get('download-custom-option').disabled, false);
assert.equal(controls.get('download_folder').value, 'custom');
assert.equal(elements.get('settings-save').disabled, false);
window.renderDownloadFolder('/tmp/Search downloads', 'cancelled');
assert.equal(elements.get('download-custom-path').textContent, '/tmp/Search downloads');
const customSettings = {...savedSettings, download_folder:'custom'};
window.renderDownloadFolder('/tmp/Search downloads', 'loaded');
window.renderSettings(customSettings, false, 'saved');
assert.equal(elements.get('settings-save').disabled, true);
window.renderDownloadFolder('/tmp/Another folder', 'selected');
assert.equal(elements.get('settings-save').disabled, false);
window.renderDownloadFolder('/tmp/Search downloads', 'loaded');
elements.get('settings-close').dispatch('click');
assert.deepEqual(messages.pop(), {type:'composer', visible:false});
assert.equal(elements.get('toolbar').inert, false);

const tabs = elements.get('tabs');
window.renderSearch({active: 0, tabs: [
  {id: 10, label: 'Новая вкладка', url: 'search:start'},
  {id: 11, label: 'Rust', url: 'https://rust-lang.org/'},
]});
assert.equal(tabs.children.length, 2);
const shortcut = options => document.keydown({preventDefault() {}, ...options});
window.searchNativeShortcut({metaKey: true, key: '['});
assert.deepEqual(messages.pop(), {type: 'back'});
window.searchNativeShortcut({metaKey: true, key: ']'});
assert.deepEqual(messages.pop(), {type: 'forward'});
shortcut({metaKey: true, shiftKey: true, key: 'T'});
assert.deepEqual(messages.pop(), {type: 'reopen'});
shortcut({ctrlKey: true, key: 't'});
assert.deepEqual(messages.pop(), {type: 'new'});

tabs.children[0].children[0].focus();
shortcut({ctrlKey: true, key: 'Tab'});
assert.deepEqual(messages.pop(), {type: 'select', id: 11});
assert.equal(document.activeElement, tabs.children[1].children[0], 'focus follows the shortcut target');
document.activeElement.blur();
shortcut({ctrlKey: true, shiftKey: true, key: 'Tab'});
assert.deepEqual(messages.pop(), {type: 'select', id: 11}, 'previous wraps to the last tab');
shortcut({metaKey: true, key: '9'});
assert.deepEqual(messages.pop(), {type: 'select', id: 11});
shortcut({ctrlKey: true, key: '2'});
assert.deepEqual(messages.pop(), {type: 'select', id: 11});
shortcut({ctrlKey: true, key: '8'});
shortcut({ctrlKey: true, key: 'Tab', isComposing: true});
assert.equal(messages.length, 0);

assert.equal(elements.get('back').disabled, true, 'new tab has no back entry');
assert.equal(elements.get('forward').disabled, true, 'new tab has no forward entry');
window.renderSearch({...window.searchState, can_back: true, can_forward: true});
assert.equal(elements.get('back').disabled, false);
assert.equal(elements.get('forward').disabled, false);
assert.equal(elements.get('reload').disabled, true, 'start page has no reload history entry');
const startState = window.searchState;
document.documentElement = new Element('html');
document.documentElement.classList.add('developer-preview');
window.renderSearch({...startState, can_back: false});
assert.equal(elements.get('back').disabled, true, 'standalone developer start has no parent window');
document.documentElement.classList.add('developer-from-search');
window.renderSearch({...startState, can_back: false});
assert.equal(elements.get('back').disabled, false, 'developer start can return to Search');
assert.equal(elements.get('back').getAttribute('aria-label'), 'Вернуться в Search');
elements.get('back').dispatch('click');
assert.deepEqual(messages.pop(), {type: 'back'});
window.renderSearch({...startState, active: 1, can_back: true});
assert.equal(elements.get('back').getAttribute('aria-label'), 'Назад');
document.documentElement = undefined;
window.renderSearch(startState);
assert.equal(tabs.children[0].children[0].getAttribute('role'), 'tab');
assert.equal(tabs.children[0].children[1].tagName, 'button', 'close is a separate keyboard button');
assert.equal(tabs.children[0].children[0].tabIndex, 0);
assert.equal(tabs.children[1].children[0].tabIndex, -1);
assert.equal(elements.get('activity').textContent, '');
const inlineAddress = elements.get('address');
assert.equal(tabs.children[0].children[2], inlineAddress, 'address is inside the selected tab');
assert.equal(inlineAddress.hidden, true);
tabs.children[0].children[0].dispatch('click');
assert.equal(document.activeElement, inlineAddress);
assert.equal(inlineAddress.hidden, false);
inlineAddress.blur();
assert.equal(inlineAddress.hidden, true);


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

document.documentElement = {classList: {contains: name => name === 'https-preview'}};
window.restoreSearchEngine('bing');
assert.equal(elements.get('engine-mark').dataset.engine, 'bing', 'startup restores the saved search engine');
assert.equal(elements.get('engine-bing').getAttribute('aria-pressed'), 'true');

const previousTabs = window.searchState.tabs;
elements.get('add').dispatch('click');
assert.deepEqual(messages.pop(), {type: 'new'});
const newTab = {id: 900, label: 'Новая вкладка', url: 'search:start'};
window.renderSearch({active: previousTabs.length, tabs: [...previousTabs, newTab]});
assert.equal(document.activeElement, address, 'new tab opens its search field immediately');
assert.equal(address.value, '', 'new tab does not reuse previous input');
assert.equal(elements.get('composer').hidden, false);
assert.deepEqual(messages.pop(), {type: 'composer', visible: true});
address.dispatch('keydown', {key: 'Escape'});
assert.equal(address.hidden, true, 'Escape closes the composer');
assert.equal(elements.get('composer').hidden, true);
assert.deepEqual(messages.splice(-2), [{type: 'composer', visible: false}, {type: 'cancel-address'}]);
assert.equal(window.searchState.tabs.at(-1).id, 900, 'Escape leaves new tab open');

elements.get('add').dispatch('click');
assert.deepEqual(messages.pop(), {type: 'new'});
window.renderSearch({active: previousTabs.length + 1, tabs: [...previousTabs, newTab, {...newTab, id: 901}]});
assert.deepEqual(messages.pop(), {type: 'composer', visible: true});
elements.get('engine-toggle').dispatch('click');
assert.equal(elements.get('engine-menu').hidden, false);
elements.get('engine-duckduckgo').dispatch('click');
assert.deepEqual(messages.pop(), {type: 'set-search-engine', engine: 'duckduckgo'});
assert.equal(elements.get('engine-menu').hidden, true);
assert.equal(elements.get('engine-mark').dataset.engine, 'duckduckgo');
assert.equal(document.activeElement, address);
address.value = 'rust async';
address.dispatch('keydown', {key: 'Enter'});
assert.deepEqual(messages.splice(-2), [
  {type: 'composer', visible: false},
  {type: 'open', value: 'rust async', engine: 'duckduckgo'},
]);

elements.get('history-open').dispatch('click');
assert.deepEqual(messages.splice(-2), [{type: 'composer', visible: true}, {type: 'history-list'}]);
window.renderHistory([
  {url: 'https://example.org/', title: 'Example Domain'},
  {url: 'https://www.rust-lang.org/', title: 'Rust'},
]);
assert.equal(elements.get('history-list').children.length, 2);
const historyFilter = elements.get('history-filter');
historyFilter.value = 'example';
historyFilter.dispatch('input');
assert.equal(elements.get('history-list').children.length, 1);
elements.get('history-list').children[0].children[1].dispatch('click');
assert.deepEqual(messages.pop(), {type: 'history-remove', value: 'https://example.org/'});
elements.get('history-clear').dispatch('click');
assert.deepEqual(messages.pop(), {type: 'history-clear'});
elements.get('history-close').dispatch('click');
assert.deepEqual(messages.pop(), {type: 'composer', visible: false});

window.renderSearch({active: 0, tabs: [{id: 902, label: 'Example Domain', url: 'https://example.org/'}]});
elements.get('bookmark-toggle').dispatch('click');
assert.deepEqual(messages.pop(), {type: 'bookmark-toggle'});
window.renderBookmarks([{url: 'https://example.org/', title: 'Example Domain'}]);
assert.equal(elements.get('bookmark-toggle').getAttribute('aria-pressed'), 'true');
elements.get('bookmarks-open').dispatch('click');
assert.deepEqual(messages.splice(-2), [{type: 'composer', visible: true}, {type: 'bookmarks-list'}]);
assert.equal(elements.get('history-clear').hidden, true);
assert.equal(elements.get('history-list').children.length, 1);
elements.get('history-list').children[0].children[1].dispatch('click');
assert.deepEqual(messages.pop(), {type: 'bookmark-remove', value: 'https://example.org/'});
elements.get('history-close').dispatch('click');
assert.deepEqual(messages.pop(), {type: 'composer', visible: false});

elements.get('download-open').dispatch('click');
assert.deepEqual(messages.pop(), {type: 'composer', visible: true});
assert.equal(elements.get('download-address').value, 'https://example.org/');
elements.get('download-form').dispatch('submit');
assert.deepEqual(messages.splice(-2), [
  {type: 'composer', visible: false},
  {type: 'download-address', value: 'https://example.org/'},
]);
window.setDownloadActive(true);
elements.get('download-open').dispatch('click');
assert.deepEqual(messages.pop(), {type:'cancel-download'});
elements.get('developer-window').dispatch('click');
assert.deepEqual(messages.pop(), {type:'developer-window'});
window.renderSearch({active:0, tabs:[{id:990, label:'Local site', url:'http://127.0.0.1:3000/'}]});
assert.equal(elements.get('inspect-page').disabled, false);
elements.get('inspect-page').dispatch('click');
assert.deepEqual(messages.pop(), {type:'inspect-page'});

console.log('Search chrome keyboard and tab controls passed');
