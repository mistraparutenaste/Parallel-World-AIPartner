const root = document.documentElement;
const chatView = document.querySelector('[data-view="chat"]');
const settingsView = document.querySelector('[data-view="settings"]');
const personalityView = document.querySelector('[data-view="personality"]');
const personalityTransition = document.querySelector('.personality-transition');
const conversationView = document.querySelector('[data-view="conversation"]');
const conversationTransition = document.querySelector('.conversation-transition');
const closeButton = document.querySelector('.mock-view--settings .close-button');
const personalityCloseButton = document.querySelector('.personality-close-button');
const conversationCloseButton = document.querySelector('.conversation-close-button');
const themeSelect = document.querySelector('#theme-select');
const mainDiamonds = [...document.querySelectorAll('[data-main-target]')];
const chatButton = document.querySelector('[data-main-target="chat"]');
const categoryDiamonds = [...document.querySelectorAll('[data-category]')];

let transitionLocked = false;
const transitionTimers = new Set();

function motionDuration(milliseconds) {
  if (root.dataset.motion === 'preview') return milliseconds;
  return window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 1 : milliseconds;
}

function scheduleTransition(callback, milliseconds) {
  const timer = window.setTimeout(() => {
    transitionTimers.delete(timer);
    callback();
  }, motionDuration(milliseconds));
  transitionTimers.add(timer);
}

function cancelTransitionTimers() {
  for (const timer of transitionTimers) window.clearTimeout(timer);
  transitionTimers.clear();
}

function setView(next) {
  const showChat = next === 'chat';
  const showSettings = next === 'settings';
  const showPersonality = next === 'personality';
  const showConversation = next === 'conversation';
  chatView.classList.remove('is-transitioning');
  chatView.classList.toggle('is-active', showChat);
  chatView.setAttribute('aria-hidden', String(!showChat));
  chatView.inert = !showChat;
  settingsView.classList.toggle('is-active', showSettings);
  settingsView.setAttribute('aria-hidden', String(!showSettings));
  settingsView.inert = !showSettings;
  personalityView.classList.toggle('is-active', showPersonality);
  personalityView.setAttribute('aria-hidden', String(!showPersonality));
  personalityView.inert = !showPersonality;
  conversationView.classList.toggle('is-active', showConversation);
  conversationView.setAttribute('aria-hidden', String(!showConversation));
  conversationView.inert = !showConversation;

  if (!showPersonality) {
    personalityView.classList.remove('is-entering');
    personalityTransition.classList.remove('is-active');
  }

  if (!showConversation) {
    conversationView.classList.remove('is-entering');
    conversationTransition.classList.remove('is-active');
  }
}

function startSettingsEntrance() {
  settingsView.classList.add('is-active');
  settingsView.setAttribute('aria-hidden', 'false');
  settingsView.inert = false;
  chatView.setAttribute('aria-hidden', 'true');
  chatView.inert = true;
  settingsView.classList.remove('is-entering');
  void settingsView.offsetWidth;
  settingsView.classList.add('is-entering');
  closeButton.focus();
  scheduleTransition(() => {
    settingsView.classList.remove('is-entering');
    transitionLocked = false;
  }, 760);
}

function startPersonalityEntrance(button) {
  const rect = button.getBoundingClientRect();
  const originX = `${rect.left + rect.width / 2}px`;
  const originY = `${rect.top + rect.height / 2}px`;
  const originSize = `${rect.width}px`;

  for (const target of [personalityView, personalityTransition]) {
    target.style.setProperty('--personality-origin-x', originX);
    target.style.setProperty('--personality-origin-y', originY);
    target.style.setProperty('--personality-origin-size', originSize);
  }

  personalityView.classList.add('is-active');
  personalityView.setAttribute('aria-hidden', 'false');
  personalityView.inert = false;
  chatView.setAttribute('aria-hidden', 'true');
  chatView.inert = true;
  chatView.classList.add('is-transitioning');
  personalityView.classList.remove('is-entering');
  personalityTransition.classList.remove('is-active');
  void personalityTransition.offsetWidth;
  personalityView.classList.add('is-entering');
  personalityTransition.classList.add('is-active');
  personalityCloseButton.focus();

  scheduleTransition(() => {
    button.classList.remove('is-confirming');
    chatView.classList.remove('is-active', 'is-transitioning');
  }, 500);

  scheduleTransition(() => {
    personalityView.classList.remove('is-entering');
    personalityTransition.classList.remove('is-active');
    transitionLocked = false;
  }, 920);
}

function startConversationEntrance(button) {
  const rect = button.getBoundingClientRect();
  const originX = `${rect.left + rect.width / 2}px`;
  const originY = `${rect.top + rect.height / 2}px`;
  const originSize = `${rect.width}px`;

  for (const target of [conversationView, conversationTransition]) {
    target.style.setProperty('--conversation-origin-x', originX);
    target.style.setProperty('--conversation-origin-y', originY);
    target.style.setProperty('--conversation-origin-size', originSize);
  }

  conversationView.classList.add('is-active');
  conversationView.setAttribute('aria-hidden', 'false');
  conversationView.inert = false;
  chatView.setAttribute('aria-hidden', 'true');
  chatView.inert = true;
  chatView.classList.add('is-transitioning');
  conversationView.classList.remove('is-entering');
  conversationTransition.classList.remove('is-active');
  void conversationTransition.offsetWidth;
  conversationView.classList.add('is-entering');
  conversationTransition.classList.add('is-active');
  conversationCloseButton.focus();

  scheduleTransition(() => {
    button.classList.remove('is-confirming');
    chatView.classList.remove('is-active', 'is-transitioning');
  }, 500);

  scheduleTransition(() => {
    conversationView.classList.remove('is-entering');
    conversationTransition.classList.remove('is-active');
    transitionLocked = false;
  }, 920);
}

function confirmMainSelection(button) {
  if (transitionLocked) return;
  transitionLocked = true;
  button.classList.add('is-confirming');

  if (button.dataset.mainTarget === 'conversation') {
    startConversationEntrance(button);
    return;
  }

  if (button.dataset.mainTarget === 'personality') {
    startPersonalityEntrance(button);
    return;
  }

  if (button.dataset.mainTarget === 'settings') {
    chatView.classList.add('is-transitioning');
    startSettingsEntrance();

    scheduleTransition(() => {
      button.classList.remove('is-confirming');
      chatView.classList.remove('is-active', 'is-transitioning');
    }, 500);
    return;
  }

  scheduleTransition(() => {
    button.classList.remove('is-confirming');
    transitionLocked = false;
  }, 500);
}

function closeFocusedView() {
  cancelTransitionTimers();
  transitionLocked = false;
  for (const button of mainDiamonds) button.classList.remove('is-confirming');
  settingsView.classList.remove('is-entering');
  setView('chat');
  chatButton.focus();
}

for (const button of mainDiamonds) {
  button.addEventListener('click', () => confirmMainSelection(button));
}

for (const button of categoryDiamonds) {
  button.addEventListener('click', () => {
    for (const item of categoryDiamonds) {
      const selected = item === button;
      item.classList.toggle('is-selected', selected);
      if (selected) item.setAttribute('aria-current', 'page');
      else item.removeAttribute('aria-current');
    }
  });
}

closeButton.addEventListener('click', () => {
  closeFocusedView();
});

personalityCloseButton.addEventListener('click', () => {
  closeFocusedView();
});

conversationCloseButton.addEventListener('click', () => {
  closeFocusedView();
});

themeSelect.addEventListener('change', (event) => {
  root.dataset.theme = event.currentTarget.value;
});

window.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') {
    if (settingsView.classList.contains('is-active')) {
      settingsView.classList.remove('is-entering');
      closeFocusedView();
    } else if (personalityView.classList.contains('is-active')) {
      closeFocusedView();
    } else if (conversationView.classList.contains('is-active')) {
      closeFocusedView();
    }
  }

  if (event.key.toLowerCase() === 't') {
    const next = root.dataset.theme === 'dark' ? 'light' : 'dark';
    root.dataset.theme = next;
    themeSelect.value = next;
  }
});
