// Small helpers for the page. Every text goes into the page as text: no
// function here, and no code of the viewer, parses text as markup.

/** Makes an element with a class and a text. The text is never read as markup. */
export function el<K extends keyof HTMLElementTagNameMap>(tag: K, className = '', text = ''): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className !== '') node.className = className;
  if (text !== '') node.textContent = text;
  return node;
}

/** Sets the text of an element when it changed. The text is never read as markup. */
export function setText(node: Element, text: string): void {
  if (node.textContent !== text) node.textContent = text;
}

/** Shows or hides an element. */
export function setShown(node: HTMLElement, shown: boolean): void {
  if (node.hidden === shown) node.hidden = !shown;
}

/** Makes a button with a label and a hint. */
export function button(label: string, hint: string, onPress: () => void, className = ''): HTMLButtonElement {
  const node = el('button', className, label);
  node.type = 'button';
  node.title = hint;
  node.setAttribute('aria-label', hint);
  node.addEventListener('click', onPress);
  return node;
}
