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

const state = { messages: [] };

function cloneMessageCard(role) {
  const fragment = messageTemplate.content.cloneNode(true);
  const card = fragment.querySelector('.message-card');
  card.querySelector('.message-role').textContent = role;
  return card;
}

function escapeHtml(value) {
  return String(value)
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;');
}

function renderMarkdownish(text) {
  const source = String(text ?? '');
  const parts = source.split(/```/g);
  let html = '';

  for (let i = 0; i < parts.length; i++) {
    if (i % 2 === 1) {
      let code = parts[i];
      const firstNewline = code.indexOf('\n');
      if (firstNewline !== -1) {
        const maybeLang = code.slice(0, firstNewline).trim();
        if (/^[A-Za-z0-9_+\-#.]+$/.test(maybeLang)) {
          code = code.slice(firstNewline + 1);
        }
      }
      html += `<pre><code>${escapeHtml(code)}</code></pre>`;
      continue;
    }

    const paragraphs = parts[i]
      .split(/\n{2,}/)
      .map((p) => p.trim())
      .filter(Boolean)
      .map((p) => `<p>${escapeHtml(p).replace(/\n/g, '<br>')}</p>`)
      .join('');

    html += paragraphs;
  }

  return html || '<p></p>';
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
    copyButton.onclick = async () => {
      await navigator.clipboard.writeText(String(nextRaw ?? ''));
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
  const chunks = String(finalText ?? '').match(/.{1,24}|\n+/gs) || [''];
  let built = '';
  for (let i = 0; i < chunks.length; i++) {
    built += chunks[i];
    messageView.update(built, rawForCopy);
    if (i < chunks.length - 1) {
      await new Promise((resolve) => setTimeout(resolve, built.length < 400 ? 16 : 8));
    }
  }
}

function summarizeToolUse(block) {
  const pretty = JSON.stringify(block.input ?? {}, null, 2);
  return `Used tool: ${block.name}\n\nArguments:\n\n\
\
\
json\n${pretty}\n\
\
\
`;
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
    max_tokens: 1024,
    system:
      'You are Claw, an offline coding assistant. Use tools only when necessary. For ordinary conversation and code explanations, answer directly in normal text with clean markdown code blocks.',
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
    await animateAssistantText(pendingAssistant, assistantText, JSON.stringify(body, null, 2));
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

refreshHealth();
setInterval(refreshHealth, 10000);
promptEl.focus();
