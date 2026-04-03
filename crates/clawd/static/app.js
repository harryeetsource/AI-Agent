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

const state = {
  messages: [],
  wrapCode: true,
};

function cloneMessageCard(role) {
  const fragment = messageTemplate.content.cloneNode(true);
  const card = fragment.querySelector('.message-card');
  card.dataset.role = role;
  card.classList.add(`role-${role}`);
  card.querySelector('.message-role').textContent = role === 'user' ? 'You' : 'Assistant';
  card.querySelector('.message-time').textContent = new Date().toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
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

function createMessageCard(role, content = '', rawForCopy = content, extraClass = '') {
  const card = cloneMessageCard(role);
  const body = card.querySelector('.message-body');
  const copyButton = card.querySelector('.copy-message');

  if (extraClass) {
    card.classList.add(extraClass);
  }

  const update = (nextContent, nextRaw = nextContent) => {
    body.innerHTML = renderMarkdownish(nextContent);
    wireCodeButtons(body);
    copyButton.onclick = async () => {
      await navigator.clipboard.writeText(String(nextRaw ?? ''));
      const original = copyButton.textContent;
      copyButton.textContent = 'Copied';
      setTimeout(() => { copyButton.textContent = original; }, 1200);
    };
    messagesEl.scrollTop = messagesEl.scrollHeight;
  };

  update(content, rawForCopy);
  messagesEl.appendChild(card);
  messagesEl.scrollTop = messagesEl.scrollHeight;

  return { card, update };
}

function appendMessage(role, content, rawForCopy = content) {
  return createMessageCard(role, content, rawForCopy);
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

function buildRequestPayload(userText) {
  const transcript = state.messages.map((item) => ({
    role: item.role,
    content: [{ type: 'text', text: item.text }],
  }));

  transcript.push({ role: 'user', content: [{ type: 'text', text: userText }] });

  return {
    model: 'local',
    max_tokens: 2048,
    system:
      'You are Claw, an offline coding assistant. Prefer clear, readable markdown with fenced code blocks. Keep code copy-friendly and well explained. When the user asks to analyze a file, directory, crate, workspace, or source tree, analyze the actual provided local source context and filesystem structure if present. Do not answer with generic shell instructions when local source context has been supplied.',
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

  appendMessage('user', text);
  state.messages.push({ role: 'user', text });
  promptEl.value = '';
  sendButton.disabled = true;
  sendButton.textContent = 'Working…';

  const pendingAssistant = createMessageCard('assistant', 'Thinking…', 'Thinking…', 'pending');
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
    setActivity('Generating response…');
    pendingAssistant.card.classList.remove('pending');
    await animateAssistantText(pendingAssistant, assistantText, assistantText);
    state.messages.push({ role: 'assistant', text: assistantText });
    setActivity('');
  } catch (error) {
    if (phaseTimer) clearTimeout(phaseTimer);
    pendingAssistant.card.classList.remove('pending');
    pendingAssistant.update(`Request failed:\n\n${error.message}`);
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
  for (const message of messagesEl.querySelectorAll('.message-body')) {
    const raw = message.textContent || '';
    // Re-render using the corresponding stored source when possible by rebuilding from card copy buffer.
  }
  // Re-render from state so code blocks pick up the new wrapping mode.
  const saved = [...state.messages];
  messagesEl.innerHTML = '';
  for (const item of saved) {
    appendMessage(item.role, item.text, item.text);
  }
});

refreshHealth();
setInterval(refreshHealth, 10000);
promptEl.focus();
