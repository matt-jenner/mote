export function createSelectionIntent() {
	let current = 0;
	return {
		begin: () => ++current,
		current: () => current,
		invalidate: () => {
			current += 1;
		},
		isCurrent: (token: number) => token === current,
	};
}
