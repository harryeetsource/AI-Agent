const messagesEl = document.getElementById('messages');
const composer = document.getElementById('composer');
const promptEl = document.getElementById('prompt');
const sendButton = document.getElementById('send-button');
const clearChatButton = document.getElementById('clear-chat');
const insertTemplateButton = document.getElementById('insert-template');
const daemonStatusEl = document.getElementById('daemon-status');
const runnerStatusEl = document.getElementById('runner-status');
const activityEl = document.getElementById('activity');
const messageTemplate = document.getElementById('message-template');
const wrapCodeCheckbox = document.getElementById('wrap-code');

const MAX_RECENT_MESSAGES = 10;
const SUMMARY_TRIGGER_MESSAGES = 14;

const state = {
  messages: [],
  wrapCode: true,
  conversationSummary: '',
};

function formatClockTimestamp(date = new Date()) {
  return date.toLocaleTimeString([], {
    hour: 'numeric',
    minute: '2-digit',
    second: '2-digit',
  });
}

function formatDurationMs(durationMs) {
  const totalSeconds = Math.max(0, Math.round(durationMs / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;

  if (minutes > 0) {
    return `thought for ${minutes}m ${seconds}s`;
  }
  return `thought for ${seconds}s`;
}

function formatTimestamp(date = new Date(), durationMs = null) {
  const base = formatClockTimestamp(date);
  if (durationMs == null) {
    return base;
  }
  return `${base} · ${formatDurationMs(durationMs)}`;
}

function cloneMessageCard(role, timestamp = new Date(), durationMs = null) {
  const fragment = messageTemplate.content.cloneNode(true);
  const card = fragment.querySelector('.message-card');
  card.dataset.role = role;
  card.classList.add(`role-${role}`);
  card.querySelector('.message-role').textContent = role === 'user' ? 'You' : 'Assistant';
  card.querySelector('.message-time').textContent = formatTimestamp(timestamp, durationMs);
  return card;
}

function escapeHtml(value) {
  return String(value)
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}

function renderInline(text) {
  let html = escapeHtml(text);
  html = html.replace(/`([^`]+)`/g, '<code class="inline-code">$1</code>');
  return html.replace(/\n/g, '<br>');
}

function splitFencedBlocks(source) {
  const text = String(source ?? '');
  const blocks = [];
  let lastIndex = 0;
  const regex = /```([A-Za-z0-9_+\-#.]+)?\n([\s\S]*?)```/g;
  let match;

  while ((match = regex.exec(text)) !== null) {
    if (match.index > lastIndex) {
      blocks.push({ type: 'text', value: text.slice(lastIndex, match.index) });
    }
    blocks.push({
      type: 'code',
      lang: (match[1] || '').trim(),
      value: match[2].replace(/\n$/, ''),
    });
    lastIndex = regex.lastIndex;
  }

  if (lastIndex < text.length) {
    blocks.push({ type: 'text', value: text.slice(lastIndex) });
  }

  return blocks;
}

function renderMarkdownish(text) {
  const blocks = splitFencedBlocks(text);
  const html = [];

  for (const block of blocks) {
    if (block.type === 'code') {
      const lang = block.lang || 'code';
      html.push(`
        <div class="code-block ${state.wrapCode ? 'wrap' : 'scroll'}">
          <div class="code-toolbar">
            <span class="code-lang">${escapeHtml(lang)}</span>
            <button type="button" class="copy-code secondary small">Copy code</button>
          </div>
          <pre><code>${escapeHtml(block.value)}</code></pre>
        </div>
      `);
      continue;
    }

    const paragraphs = block.value
      .split(/\n{2,}/)
      .map((p) => p.trim())
      .filter(Boolean)
      .map((p) => `<p>${renderInline(p)}</p>`)
      .join('');

    if (paragraphs) {
      html.push(paragraphs);
    }
  }

  return html.join('') || '<p></p>';
}

function wireCodeButtons(body) {
  for (const button of body.querySelectorAll('.copy-code')) {
    button.addEventListener('click', async () => {
      const code = button.closest('.code-block')?.querySelector('code')?.textContent ?? '';
      await navigator.clipboard.writeText(code);
      const original = button.textContent;
      button.textContent = 'Copied';
      setTimeout(() => { button.textContent = original; }, 1200);
    });
  }
}

function buildFeedbackBar(message) {
  if (
    message.role !== 'assistant' ||
    message.feedbackSubmitted ||
    message.responseComplete !== true ||
    message.isError === true
  ) {
    return '';
  }

  return `
    <div class="feedback-bar">
      <span class="feedback-label">Was this helpful?</span>
      <button type="button" class="feedback-btn" data-feedback="up" title="Helpful">👍</button>
      <button type="button" class="feedback-btn" data-feedback="down" title="Not helpful">👎</button>
    </div>
    <div class="feedback-note hidden">
      <textarea class="feedback-note-input" rows="3" placeholder="What was wrong or missing?"></textarea>
      <div class="feedback-note-actions">
        <button type="button" class="feedback-send-note secondary small">Send</button>
        <button type="button" class="feedback-skip-note secondary small">Skip note</button>
        <button type="button" class="feedback-cancel-note secondary small">Cancel</button>
      </div>
    </div>
  `;
}

async function submitFeedback(message, helpful, note = '') {
  const payload = {
    prompt: message.promptText ?? '',
    response: message.text ?? '',
    repo_path: message.repoPath ?? null,
    files_reviewed: message.filesReviewed ?? [],
    helpful,
    note: note.trim(),
    duration_ms: message.durationMs ?? null,
  };

  const response = await fetch('/feedback', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  });

  const body = await response.json().catch(() => ({}));
  if (!response.ok) {
    throw new Error(body.error || `HTTP ${response.status}`);
  }
}

function wireFeedback(card, message) {
  const bar = card.querySelector('.feedback-bar');
  if (!bar) return;

  const noteWrap = card.querySelector('.feedback-note');
  const noteInput = card.querySelector('.feedback-note-input');
  const upBtn = card.querySelector('[data-feedback="up"]');
  const downBtn = card.querySelector('[data-feedback="down"]');
  const sendNoteBtn = card.querySelector('.feedback-send-note');
  const skipNoteBtn = card.querySelector('.feedback-skip-note');
  const cancelNoteBtn = card.querySelector('.feedback-cancel-note');

  const setButtonsDisabled = (disabled) => {
    if (upBtn) upBtn.disabled = disabled;
    if (downBtn) downBtn.disabled = disabled;
    if (sendNoteBtn) sendNoteBtn.disabled = disabled;
    if (skipNoteBtn) skipNoteBtn.disabled = disabled;
    if (cancelNoteBtn) cancelNoteBtn.disabled = disabled;
    if (noteInput) noteInput.disabled = disabled;
  };

  const markSubmitted = (label) => {
    if (bar) {
      bar.innerHTML = `<span class="feedback-saved">${escapeHtml(label)}</span>`;
    }
    if (noteWrap) {
      noteWrap.classList.add('hidden');
    }
    message.feedbackSubmitted = true;
  };

  upBtn?.addEventListener('click', async () => {
    setButtonsDisabled(true);
    try {
      await submitFeedback(message, true, '');
      markSubmitted('Saved feedback: helpful');
    } catch (error) {
      setButtonsDisabled(false);
      alert(`Feedback failed: ${error.message}`);
    }
  });

  downBtn?.addEventListener('click', () => {
    if (noteWrap) noteWrap.classList.remove('hidden');
    if (noteInput) noteInput.focus();
  });

  sendNoteBtn?.addEventListener('click', async () => {
    setButtonsDisabled(true);
    try {
      await submitFeedback(message, false, noteInput?.value || '');
      markSubmitted('Saved feedback: not helpful');
    } catch (error) {
      setButtonsDisabled(false);
      alert(`Feedback failed: ${error.message}`);
    }
  });

  skipNoteBtn?.addEventListener('click', async () => {
    setButtonsDisabled(true);
    try {
      await submitFeedback(message, false, '');
      markSubmitted('Saved feedback: not helpful');
    } catch (error) {
      setButtonsDisabled(false);
      alert(`Feedback failed: ${error.message}`);
    }
  });

  cancelNoteBtn?.addEventListener('click', () => {
    if (noteWrap) noteWrap.classList.add('hidden');
  });
}

function createMessageCard(
  role,
  content = '',
  rawForCopy = content,
  extraClass = '',
  timestamp = new Date(),
  durationMs = null,
  messageMeta = null
) {
  const card = cloneMessageCard(role, timestamp, durationMs);
  const body = card.querySelector('.message-body');
  const copyButton = card.querySelector('.copy-message');
  const timeEl = card.querySelector('.message-time');

  if (extraClass) {
    card.classList.add(extraClass);
  }

  const update = (nextContent, nextRaw = nextContent) => {
    const feedbackHtml = messageMeta ? buildFeedbackBar(messageMeta) : '';
    body.innerHTML = `${renderMarkdownish(nextContent)}${feedbackHtml}`;
    wireCodeButtons(body);
    if (messageMeta) {
      wireFeedback(card, messageMeta);
    }
    copyButton.onclick = async () => {
      await navigator.clipboard.writeText(String(nextRaw ?? ''));
      const original = copyButton.textContent;
      copyButton.textContent = 'Copied';
      setTimeout(() => { copyButton.textContent = original; }, 1200);
    };
    messagesEl.scrollTop = messagesEl.scrollHeight;
  };

  const setTimestamp = (date = new Date(), durationMsValue = null) => {
    timeEl.textContent = formatTimestamp(date, durationMsValue);
  };

  update(content, rawForCopy);
  messagesEl.appendChild(card);
  messagesEl.scrollTop = messagesEl.scrollHeight;

  return { card, update, setTimestamp };
}

function appendMessage(
  role,
  content,
  rawForCopy = content,
  timestamp = new Date(),
  durationMs = null,
  messageMeta = null
) {
  return createMessageCard(role, content, rawForCopy, '', timestamp, durationMs, messageMeta);
}

function setActivity(message) {
  if (!message) {
    activityEl.textContent = '';
    activityEl.classList.add('hidden');
    return;
  }
  activityEl.textContent = message;
  activityEl.classList.remove('hidden');
}

async function animateAssistantText(messageView, finalText, rawForCopy) {
  const chunks = String(finalText ?? '').match(/.{1,28}|\n+/gs) || [''];
  let built = '';
  for (let i = 0; i < chunks.length; i++) {
    built += chunks[i];
    messageView.update(built, rawForCopy);
    if (i < chunks.length - 1) {
      await new Promise((resolve) => setTimeout(resolve, built.length < 500 ? 14 : 7));
    }
  }
}

function summarizeToolUse(block) {
  const pretty = JSON.stringify(block.input ?? {}, null, 2);
  return `Used tool: ${block.name}\n\n\`\`\`json\n${pretty}\n\`\`\``;
}

function extractReviewedFiles(text) {
  const matches = [...String(text ?? '').matchAll(/FILE:\s+([^\n]+)/g)];
  return matches.map((m) => m[1].trim());
}

function detectRepoPathFromPrompt(text) {
  const match = String(text ?? '').match(/([A-Za-z]:\\[^\n]+|\/[^\n]+)/);
  return match ? match[1].trim() : null;
}

function isFullFileRewritePrompt(text) {
  const lower = String(text ?? '').toLowerCase();
  const hasFilePath = /([a-z]:\\[^\n]+|\/[^\n]+\.[a-z0-9_]+)/i.test(lower);
  const rewriteIntent =
    /full source code fix|full rewrite|complete replacement|return the full file|provide the full file|rewrite this file|provide a version of this file|full file rewrite|replacement file|version of this file|modify this file|update this file|correct this file|repair this file/.test(lower);

  return hasFilePath && rewriteIntent;
}

function responseToDisplay(response) {
  const textParts = [];
  for (const block of response.content || []) {
    if (block.type === 'text' && block.text) {
      textParts.push(block.text);
    } else if (block.type === 'tool_use') {
      textParts.push(summarizeToolUse(block));
    } else if (block.type === 'thinking' && block.thinking) {
      textParts.push(`Thinking:\n\n${block.thinking}`);
    }
  }
  return textParts.join('\n\n').trim();
}

function summarizeOlderMessages(messages) {
  if (!messages.length) return '';

  const recent = messages.slice(-8);
  const lines = recent.map((msg, idx) => {
    const label = msg.role === 'user' ? 'User' : 'Assistant';
    const text = String(msg.text ?? '').replace(/\s+/g, ' ').trim();
    const compact = text.length > 220 ? `${text.slice(0, 220)}…` : text;
    return `${idx + 1}. ${label}: ${compact}`;
  });

  return [
    'Earlier conversation summary:',
    ...lines,
  ].join('\n');
}

function maybeRefreshConversationSummary() {
  if (state.messages.length < SUMMARY_TRIGGER_MESSAGES) {
    return;
  }

  const older = state.messages.slice(0, Math.max(0, state.messages.length - MAX_RECENT_MESSAGES));
  if (!older.length) {
    return;
  }

  const summary = summarizeOlderMessages(older);
  if (summary) {
    state.conversationSummary = summary;
  }
}

function buildRequestPayload(userText) {
  maybeRefreshConversationSummary();

  const recentMessages = state.messages.slice(-MAX_RECENT_MESSAGES);
  const transcript = [];

  if (state.conversationSummary) {
    transcript.push({
      role: 'system',
      content: [{
        type: 'text',
        text: state.conversationSummary,
      }],
    });
  }

  for (const item of recentMessages) {
    transcript.push({
      role: item.role,
      content: [{ type: 'text', text: item.text }],
    });
  }

  transcript.push({ role: 'user', content: [{ type: 'text', text: userText }] });

  const maxTokens = isFullFileRewritePrompt(userText) ? 12000 : 2048;

  return {
    model: 'local',
    max_tokens: maxTokens,
    system:
      'You are Claw, an offline coding assistant. Prefer clear, readable markdown with fenced code blocks. Keep code copy-friendly and well explained. When the user asks to analyze a file, directory, crate, workspace, or source tree, analyze the actual provided local source context and filesystem structure if present. When the user requests a full file rewrite or complete replacement, prefer returning the complete replacement file without unnecessary preamble.',
    messages: transcript,
    stream: false,
  };
}

async function refreshHealth() {
  try {
    const response = await fetch('/health');
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const data = await response.json();
    daemonStatusEl.textContent = 'OK';
    runnerStatusEl.textContent = data.runner_messages_url || 'Configured';
  } catch (error) {
    daemonStatusEl.textContent = 'Down';
    runnerStatusEl.textContent = error.message;
  }
}

async function submitPrompt(event) {
  event.preventDefault();
  const text = promptEl.value.trim();
  if (!text) return;

  const userTimestamp = new Date();
  const userMessage = {
    role: 'user',
    text,
    timestamp: userTimestamp.toISOString(),
    durationMs: null,
  };

  appendMessage('user', text, text, userTimestamp, null, userMessage);
  state.messages.push(userMessage);

  promptEl.value = '';
  sendButton.disabled = true;
  sendButton.textContent = 'Working…';

  const assistantStartedAt = new Date();
  const assistantMessage = {
    role: 'assistant',
    text: 'Thinking…',
    timestamp: assistantStartedAt.toISOString(),
    durationMs: null,
    promptText: text,
    repoPath: detectRepoPathFromPrompt(text),
    filesReviewed: [],
    feedbackSubmitted: false,
    responseComplete: false,
    isError: false,
  };

  const pendingAssistant = createMessageCard(
    'assistant',
    'Thinking…',
    'Thinking…',
    'pending',
    assistantStartedAt,
    null,
    assistantMessage
  );

  let phaseTimer = null;

  try {
    setActivity('Thinking…');
    phaseTimer = setTimeout(() => setActivity('Generating response…'), 900);

    const response = await fetch('/v1/messages', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(buildRequestPayload(text)),
    });

    const body = await response.json();
    if (!response.ok) {
      throw new Error(body.error || `HTTP ${response.status}`);
    }

    if (phaseTimer) clearTimeout(phaseTimer);

    const assistantText = responseToDisplay(body) || 'No response text returned.';
    const assistantCompletedAt = new Date();
    const durationMs = assistantCompletedAt.getTime() - assistantStartedAt.getTime();

    assistantMessage.text = assistantText;
    assistantMessage.timestamp = assistantCompletedAt.toISOString();
    assistantMessage.durationMs = durationMs;
    assistantMessage.filesReviewed = extractReviewedFiles(assistantText);
    assistantMessage.responseComplete = true;
    assistantMessage.isError = false;

    setActivity('Generating response…');
    pendingAssistant.card.classList.remove('pending');
    pendingAssistant.setTimestamp(assistantCompletedAt, durationMs);
    await animateAssistantText(pendingAssistant, assistantText, assistantText);

    state.messages.push(assistantMessage);
    setActivity('');
  } catch (error) {
    if (phaseTimer) clearTimeout(phaseTimer);

    const assistantCompletedAt = new Date();
    const durationMs = assistantCompletedAt.getTime() - assistantStartedAt.getTime();
    const failureText = `Request failed:\n\n${error.message}`;

    assistantMessage.text = failureText;
    assistantMessage.timestamp = assistantCompletedAt.toISOString();
    assistantMessage.durationMs = durationMs;
    assistantMessage.responseComplete = false;
    assistantMessage.isError = true;

    pendingAssistant.card.classList.remove('pending');
    pendingAssistant.setTimestamp(assistantCompletedAt, durationMs);
    pendingAssistant.update(failureText, failureText);

    state.messages.push(assistantMessage);
    setActivity('');
  } finally {
    sendButton.disabled = false;
    sendButton.textContent = 'Send';
    promptEl.focus();
  }
}

composer.addEventListener('submit', submitPrompt);

promptEl.addEventListener('keydown', (event) => {
  if (event.key === 'Enter' && event.ctrlKey) {
    composer.requestSubmit();
    event.preventDefault();
  }
});

clearChatButton.addEventListener('click', () => {
  state.messages.length = 0;
  state.conversationSummary = '';
  messagesEl.innerHTML = '';
  setActivity('');
  promptEl.focus();
});

insertTemplateButton.addEventListener('click', () => {
  const template = [
    'Please help me with this Rust code:',
    '```rust',
    'fn main() {',
    '    println!("hello");',
    '}',
    '```',
  ].join('\n');

  promptEl.value = template;
  promptEl.focus();
});

wrapCodeCheckbox.addEventListener('change', () => {
  state.wrapCode = wrapCodeCheckbox.checked;

  const saved = [...state.messages];
  messagesEl.innerHTML = '';

  for (const item of saved) {
    appendMessage(
      item.role,
      item.text,
      item.text,
      item.timestamp ? new Date(item.timestamp) : new Date(),
      item.durationMs ?? null,
      item
    );
  }
});

refreshHealth();
setInterval(refreshHealth, 10000);
promptEl.focus();
