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

function loadSession(app, id) {
  app.applyListResponse('sessions', {
    ok: true,
    data: {
      sessions: [{
        id: id,
        agent_id: 'lin',
        status: 'active',
        work_item_id: 1,
        last_checkpoint_id: 7,
        started_at: Date.now(),
      }],
    },
  }, 'sessions', 'session.list');
}

test('compaction inspect expands occupancy and guidance from CLI JSON', () => {
  const app = loadApp();
  const $ = (id) => document.getElementById(id);
  loadSession(app, 11);
  app.applySessionShow(11, {
    ok: true,
    command: 'awr session show 11',
    data: {
      checkpoint: { id: 7, next_action: 'Wire the panel', open_loops: [] },
      claims: [],
    },
  });
  app.applyCompactionInspect(11, {
    ok: true,
    command: 'awr session compaction inspect --session 11',
    data: {
      ok: true,
      read_only: true,
      session: 11,
      state: 'continue',
      observation_event_id: 31,
      post_compaction_percent: 42.0,
      previous_delta_percentage_points: null,
      policy: { post_compaction_threshold_percent: 50 },
      guidance: {
        when: 'Same model/window; completed automatic compaction is below threshold',
        basis: 'Host-reported post-compaction occupancy 42.0%; threshold 50% (heuristic)',
        next_action: 'Continue the current task with native compaction enabled',
        recheck: 'Next completed compaction or changed model/window',
      },
      native_compaction: 'unchanged',
      session_switch_performed: false,
    },
  });
  app.renderSessions();
  assert.equal($('sessionPanel').hidden, false);
  assert.equal($('sessionDetail').hidden, false);
  const text = $('sessionDetail').textContent;
  assert.ok(text.includes('Wire the panel'));
  assert.ok(text.includes('continue'));
  assert.ok(text.includes('42.0%'));
  assert.ok(text.includes('50%'));
  assert.ok(text.includes('Continue the current task with native compaction enabled'));
  assert.ok(text.includes(i18n.t('ui.compaction_read_only')));
  assert.ok(text.includes(i18n.t('ui.compaction_native_unchanged')));
  assert.ok(text.includes(i18n.t('ui.compaction_no_switch')));
  assert.ok(!text.includes('--include-observation'));
});

test('missing occupancy is not shown as zero', () => {
  const app = loadApp();
  const $ = (id) => document.getElementById(id);
  loadSession(app, 11);
  app.applySessionShow(11, {
    ok: true,
    data: { checkpoint: { id: 7, next_action: 'Keep going', open_loops: [] }, claims: [] },
  });
  app.applyCompactionInspect(11, {
    ok: true,
    command: 'awr session compaction inspect --session 11',
    data: {
      read_only: true,
      session: 11,
      state: 'measurement_missing',
      post_compaction_percent: null,
      policy: { post_compaction_threshold_percent: 50 },
      guidance: {
        when: 'Post-compaction full-request usage or effective window is unknown',
        basis: 'Summary length and cumulative billed tokens cannot establish occupancy',
        next_action: 'Continue existing workflow; have the host report available full-request metrics without guessing',
        recheck: 'Next completed compaction with comparable host measurements',
      },
      native_compaction: 'unchanged',
      session_switch_performed: false,
    },
  });
  app.renderSessions();
  const text = $('sessionDetail').textContent;
  assert.ok(text.includes('measurement_missing'));
  assert.ok(text.includes(i18n.t('ui.compaction_occupancy_missing')));
  assert.ok(!text.includes('0.0%'));
  assert.ok(!text.includes('occupancy 0'));
});

test('compaction inspect Unsupported keeps session show visible', () => {
  const app = loadApp();
  const $ = (id) => document.getElementById(id);
  loadSession(app, 11);
  app.applySessionShow(11, {
    ok: true,
    command: 'awr session show 11',
    data: {
      checkpoint: { id: 7, next_action: 'Wire the panel', open_loops: ['Confirm JSON'] },
      claims: [],
    },
  });
  app.applyCompactionInspect(11, {
    ok: false,
    command: 'awr session compaction inspect --session 11',
    error: { code: 'Unsupported', message: 'operation is not implemented' },
  });
  app.renderSessions();
  assert.equal($('sessionPanel').hidden, false);
  assert.ok($('sessionList').textContent.includes('lin'));
  assert.equal($('sessionDetail').hidden, false);
  const text = $('sessionDetail').textContent;
  assert.ok(text.includes('Wire the panel'));
  assert.ok(text.includes('Confirm JSON'));
  assert.ok(text.includes('Unsupported'));
});
