import { translate, type Locale, type Translator } from "./i18n/messages.ts";

type FormField = HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement;
type ValidationKey = "validation.required" | "validation.selectRequired" | "validation.invalidNumber" | "validation.invalid";

function isFormField(element: Element): element is FormField {
  return element instanceof HTMLInputElement || element instanceof HTMLSelectElement || element instanceof HTMLTextAreaElement;
}

export function clearLocalizedValidity(target: EventTarget) {
  if (target instanceof Element && isFormField(target)) {
    target.setCustomValidity("");
    delete target.dataset.uiValidationKey;
  }
}

function setLocalizedValidity(field: FormField, key: ValidationKey, t: Translator) {
  field.dataset.uiValidationKey = key;
  field.setCustomValidity(t(key));
}

export function refreshLocalizedValidity(locale: Locale) {
  for (const element of document.querySelectorAll("[data-ui-validation-key]")) {
    if (!isFormField(element)) continue;
    const key = element.dataset.uiValidationKey;
    if (key === "validation.required" || key === "validation.selectRequired" || key === "validation.invalidNumber" || key === "validation.invalid") {
      element.setCustomValidity(translate(locale, key));
    }
  }
}

export function validateLocalizedForm(form: HTMLFormElement, t: Translator): boolean {
  const fields = [...form.elements].filter((element): element is FormField => element instanceof Element && isFormField(element));
  for (const field of fields) clearLocalizedValidity(field);
  for (const field of fields) {
    if (field.disabled || !field.willValidate) continue;
    if (field.required && !field.value.trim()) {
      setLocalizedValidity(field, field instanceof HTMLSelectElement ? "validation.selectRequired" : "validation.required", t);
    } else if (field.validity.badInput || field.validity.rangeUnderflow || field.validity.stepMismatch || field.validity.rangeOverflow) {
      setLocalizedValidity(field, "validation.invalidNumber", t);
    } else if (!field.validity.valid) {
      setLocalizedValidity(field, "validation.invalid", t);
    }
    if (!field.validity.valid) {
      field.reportValidity();
      return false;
    }
  }
  return true;
}
