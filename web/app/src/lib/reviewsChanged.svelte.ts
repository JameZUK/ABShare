// A signal that someone's review of a book changed somewhere in the app (the "How was it?"
// prompt, say), so an open book page reloads its reviews and rating.
export const reviewsChanged = $state({ book: '', n: 0 });

export function noteReviewChanged(book: string) {
	reviewsChanged.book = book;
	reviewsChanged.n++;
}
