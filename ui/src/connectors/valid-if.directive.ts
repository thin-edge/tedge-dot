import { Directive, effect, input } from '@angular/core';
import { type AbstractControl, NG_VALIDATORS, type ValidationErrors, type Validator } from '@angular/forms';

/**
 * Makes an ngModel field invalid when `tdotValidIf` says its value is not valid. c8y-form-group
 * styles a field with ngModel from the field's own validity and ignores its `hasError` input, so a
 * rule checked outside Angular forms would leave a refused value looking valid.
 */
@Directive({
  selector: '[tdotValidIf]',
  providers: [{ provide: NG_VALIDATORS, useExisting: ValidIfDirective, multi: true }],
})
export class ValidIfDirective implements Validator {
  /** Whether a value is valid; a new function, such as for another kind of file, checks again. */
  readonly tdotValidIf = input.required<(value: string) => boolean>();

  private revalidate?: () => void;

  constructor() {
    effect(() => {
      this.tdotValidIf();
      this.revalidate?.();
    });
  }

  validate(control: AbstractControl): ValidationErrors | null {
    return this.tdotValidIf()(String(control.value ?? '')) ? null : { tdotValidIf: true };
  }

  registerOnValidatorChange(fn: () => void): void {
    this.revalidate = fn;
  }
}
