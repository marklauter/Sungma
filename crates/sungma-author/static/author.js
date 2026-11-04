// Copies a button's data-copy value to the clipboard.
document.addEventListener("click", (event) => {
  const button = event.target.closest("[data-copy]");
  if (button) {
    navigator.clipboard.writeText(button.dataset.copy);
  }
});
