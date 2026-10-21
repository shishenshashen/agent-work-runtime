'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { install } = require('./fixtures/dom-stub.js');
const i18n = require('../public/i18n.js');

function loadApp() {
  install();
  delete require.cache[require.resolve('../public/app.js')];
  return require('../public/app.js');
}

function loadEvents(app, items) {
  app.applyListResponse('events', {
    ok: true,
    data: { events: items },
  }, 'events', 'event.history');
}

test('event show expands metadata and payload from CLI JSON', () => {
  const app = loadApp();
  const $ = (id) => document.getElementById(id);
  loadEvents(app, [{
    id: 21,
    type: 'session_started',
    summary: 'session started',
    importance: 'high',
    session_id: 11,
    work_item_id: 1,
    created_at: Date.now(),
  }]);
  app.applyEventShow(21, {
    ok: true,
    command: 'awr event show 21 --full --max-bytes 65536',
    data: {
      event: {
        id: 21,
        work_item_id: 1,
        session_id: 11,
        branch_id: 1,
        event_type: 'session_started',
        importance: 'high',
        summary: 'session started',
        payload: { agent_id: 'lin', provider: 'cursor' },
        created_at: 1700000000000,
      },
      payload_included: true,
    },
  });
  app.renderEvents();
  assert.equal($('eventPanel').hidden, false);
  assert.equal($('eventDetail').hidden, false);
  const text = $('eventDetail').textContent;
  assert.ok(text.includes('session started'));
  assert.ok(text.includes('session_started'));
  assert.ok(text.includes('session=11'));
  assert.ok(text.includes('work=1'));
  assert.ok(text.includes('lin'));
  assert.ok(text.includes('cursor'));
  assert.ok(text.includes(i18n.t('ui.event_payload')));
  assert.equal($('eventList').find((n) => n.tagName === 'LI').getAttribute('aria-selected'), 'true');
});

test('event show without --full keeps the payload omitted', () => {
  const app = loadApp();
  const $ = (id) => document.getElementById(id);
  loadEvents(app, [{ id: 21, type: 'session_started', summary: 'session started' }]);
  app.applyEventShow(21, {
    ok: true,
    command: 'awr event show 21',
    data: {
      event: {
        id: 21,
        type: 'session_started',
        summary: 'session started',
        session_id: 11,
      },
      payload_included: false,
    },
  });
  app.renderEvents();
  assert.ok($('eventDetail').textContent.includes(i18n.t('ui.event_payload_omitted')));
  assert.ok(!$('eventDetail').textContent.includes('agent_id'));
});

test('event show truncates oversized payload for display', () => {
  const app = loadApp();
  const $ = (id) => document.getElementById(id);
  loadEvents(app, [{ id: 21, type: 'note', summary: 'long payload' }]);
  app.applyEventShow(21, {
    ok: true,
    data: {
      event: {
        id: 21,
        event_type: 'note',
        summary: 'long payload',
        payload: { body: 'x'.repeat(5000) },
      },
      payload_included: true,
    },
  });
  app.renderEvents();
  const text = $('eventDetail').textContent;
  assert.ok(text.includes(i18n.t('ui.event_payload_truncated')));
  assert.ok(text.length < 6000);
});

test('event show Unsupported keeps the list visible', () => {
  const app = loadApp();
  const $ = (id) => document.getElementById(id);
  loadEvents(app, [{
    id: 21,
    type: 'session_started',
    summary: 'session started',
    session_id: 11,
  }]);
  app.applyEventShow(21, {
    ok: false,
    command: 'awr event show 21 --full --max-bytes 65536',
    error: { code: 'Unsupported', message: 'operation is not implemented' },
  });
  app.renderEvents();
  assert.equal($('eventPanel').hidden, false);
  assert.ok($('eventList').textContent.includes('session started'));
  assert.equal($('eventDetail').hidden, false);
  assert.ok($('eventDetail').textContent.includes('Unsupported'));
});
