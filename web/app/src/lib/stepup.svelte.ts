// Step-up (SPEC §5.5): account changes need a fresh confirmation. `withStepUp` runs an action
// and, if the server asks, shows the confirmation sheet and tries once more.
import { ApiError } from '$lib/api/http';

class StepUp {
	open = $state(false);
	#resolve: ((ok: boolean) => void) | null = null;

	ask(): Promise<boolean> {
		this.#resolve?.(false);
		this.open = true;
		return new Promise((resolve) => (this.#resolve = resolve));
	}

	done(ok: boolean) {
		this.open = false;
		this.#resolve?.(ok);
		this.#resolve = null;
	}
}

export const stepUp = new StepUp();

export async function withStepUp<T>(fn: () => Promise<T>): Promise<T> {
	try {
		return await fn();
	} catch (e) {
		if (e instanceof ApiError && e.code === 'step_up_required' && (await stepUp.ask())) return fn();
		throw e;
	}
}
