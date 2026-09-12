import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PhotoServiceProvider } from "../app/PhotoServiceContext";
import { PickToast } from "../components/PickToast";
import { createInMemoryPhotoService } from "../services/inMemoryPhotoService";
import type { PhotoService, WallAsset } from "../services/photoService";
import {
	PickListProvider,
	pickOriginFromSavedFolders,
	usePickListOrigin,
} from "./PickListContext";
import type { PickListSnapshot } from "./pickList";
import { createPickListStore } from "./usePickList";

const asset = (id: string): WallAsset => ({
	id,
	displayName: `${id}.jpg`,
	mediaKind: "jpeg",
	provisionalOrder: 0,
	capturedAtUtc: null,
	dateState: "settled",
	width: 1600,
	height: 1200,
	representativeRgb: null,
	shapeState: "ready",
	availability: "available",
	warning: null,
	wallThumbnail: null,
	screenPreview: null,
	rating: null,
});

const origin = { sourceFolderId: "folder-a", sourceLabel: "Holiday" };
const reference = (item: WallAsset) => ({
	assetId: item.id,
	sourceFolderId: origin.sourceFolderId,
	sourceLabel: origin.sourceLabel,
});
const snapshot = (
	items: WallAsset[],
	revision = 0,
	persistenceError: string | null = null,
): PickListSnapshot => ({
	revision,
	items: items.map((item) => ({ ...reference(item), asset: item })),
	persistenceError,
});

function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (reason?: unknown) => void;
	const promise = new Promise<T>((resolvePromise, rejectPromise) => {
		resolve = resolvePromise;
		reject = rejectPromise;
	});
	return { promise, reject, resolve };
}

afterEach(() => {
	vi.useRealTimers();
});

describe("pick list controller", () => {
	it("starts with the synchronous snapshot and accepts the loaded snapshot", async () => {
		const first = asset("first");
		const loaded = asset("loaded");
		const load = deferred<PickListSnapshot>();
		const memory = createInMemoryPhotoService();
		const service: PhotoService = {
			...memory,
			getPicks: () => snapshot([first], 1),
			loadPicks: () => load.promise,
		};
		const store = createPickListStore(service);

		expect(store.getState().count).toBe(1);
		expect(store.getState().isPicked(first.id)).toBe(true);
		store.start();
		load.resolve(snapshot([loaded], 2));
		await load.promise;
		await Promise.resolve();

		expect(store.getState().snapshot.items.map((item) => item.assetId)).toEqual(
			["loaded"],
		);
		expect(store.getState().isPicked(first.id)).toBe(false);
	});

	it("suppresses a duplicate toggle while showing the optimistic count", async () => {
		const added = asset("added");
		const gate = deferred<PickListSnapshot>();
		const memory = createInMemoryPhotoService();
		const addPick = vi.fn(() => gate.promise);
		const service: PhotoService = { ...memory, addPick };
		const store = createPickListStore(service);
		store.start();

		const first = store.getState().toggle(added, origin);
		const duplicate = store.getState().toggle(added, origin);

		expect(store.getState().count).toBe(1);
		expect(store.getState().isPicked(added.id)).toBe(true);
		expect(addPick).toHaveBeenCalledOnce();
		gate.resolve(snapshot([added], 1));
		await Promise.all([first, duplicate]);
		expect(store.getState().count).toBe(1);
	});

	it("rolls back an optimistic toggle when persistence fails", async () => {
		const rejected = asset("rejected");
		const memory = createInMemoryPhotoService();
		const service: PhotoService = {
			...memory,
			addPick: async () => {
				throw new Error("Disk is read-only");
			},
		};
		const store = createPickListStore(service);
		store.start();

		const mutation = store.getState().toggle(rejected, origin);
		expect(store.getState().count).toBe(1);
		await expect(mutation).rejects.toThrow("Disk is read-only");

		expect(store.getState().count).toBe(0);
		expect(store.getState().isPicked(rejected.id)).toBe(false);
		expect(store.getState().announcement).toBe("Couldn't update picks");
	});

	it("offers Clear Undo for exactly five seconds", async () => {
		vi.useFakeTimers();
		const first = asset("first");
		const service = createInMemoryPhotoService({ wallAssets: [first] });
		await service.addPick(reference(first));
		const store = createPickListStore(service);
		store.start();

		await store.getState().clear();
		expect(store.getState().count).toBe(0);
		expect(store.getState().toast?.action?.label).toBe("Undo");

		vi.advanceTimersByTime(4_999);
		expect(store.getState().toast?.action?.label).toBe("Undo");
		vi.advanceTimersByTime(1);
		expect(store.getState().toast).toBeNull();

		await store.getState().undoClear();
		expect(store.getState().count).toBe(0);
	});

	it("restores cleared entries before picks made after Clear", async () => {
		vi.useFakeTimers();
		const first = asset("first");
		const second = asset("second");
		const later = asset("later");
		const service = createInMemoryPhotoService({
			wallAssets: [first, second, later],
		});
		await service.addPick(reference(first));
		await service.addPick(reference(second));
		const store = createPickListStore(service);
		store.start();

		await store.getState().clear();
		await store.getState().toggle(later, origin);
		await store.getState().undoClear();

		expect(store.getState().snapshot.items.map((item) => item.assetId)).toEqual(
			["first", "second", "later"],
		);
		expect(store.getState().announcement).toBe("Picks restored");
	});

	it("exposes a browser persistence warning", () => {
		const warning =
			"Browser storage is unavailable. Picks may not survive reload.";
		const memory = createInMemoryPhotoService();
		const service: PhotoService = {
			...memory,
			getPicks: () => snapshot([], 0, warning),
		};
		const store = createPickListStore(service);

		expect(store.getState().snapshot.persistenceError).toBe(warning);
		expect(store.getState().announcement).toBe(warning);
		expect(store.getState().toast?.message).toBe(warning);
	});

	it("cleans up the service subscription and Undo timer", async () => {
		vi.useFakeTimers();
		const first = asset("first");
		const memory = createInMemoryPhotoService({ wallAssets: [first] });
		await memory.addPick(reference(first));
		const unsubscribe = vi.fn();
		const service: PhotoService = {
			...memory,
			watchPicks: () => unsubscribe,
		};
		const store = createPickListStore(service);
		const stop = store.start();
		await store.getState().clear();

		stop();
		vi.advanceTimersByTime(5_000);

		expect(unsubscribe).toHaveBeenCalledOnce();
		expect(store.getState().toast?.action?.label).toBe("Undo");
	});
});

describe("pick list provider", () => {
	it("derives origin from the active saved-folder identity and label", () => {
		expect(
			pickOriginFromSavedFolders({
				activeEntryId: "entry-a",
				hasOpenedFolder: true,
				persistenceError: null,
				access: {},
				entries: [
					{
						id: "entry-a",
						folderId: "stable-folder-id",
						name: "Camera",
						displayPath: "/photos/camera",
						customLabel: "Summer selects",
					},
				],
			}),
		).toEqual({
			sourceFolderId: "stable-folder-id",
			sourceLabel: "Summer selects",
		});
	});

	it("renders a polite live region, visible warning, and saved-folder origin", () => {
		const warning =
			"Browser storage is unavailable. Picks may not survive reload.";
		const memory = createInMemoryPhotoService();
		const service: PhotoService = {
			...memory,
			getPicks: () => snapshot([], 0, warning),
		};
		function OriginProbe() {
			const current = usePickListOrigin();
			return (
				<span>{`${current?.sourceFolderId}:${current?.sourceLabel}`}</span>
			);
		}

		const markup = renderToStaticMarkup(
			<PhotoServiceProvider service={service}>
				<PickListProvider origin={origin}>
					<OriginProbe />
				</PickListProvider>
			</PhotoServiceProvider>,
		);

		expect(markup).toContain('aria-live="polite"');
		expect(markup).toContain('aria-label="Pick storage warning"');
		expect(markup).toContain(warning);
		expect(markup).toContain("folder-a:Holiday");
	});

	it("keeps the Undo toast action focusable without requesting focus", () => {
		const markup = renderToStaticMarkup(
			<PickToast
				toast={{
					id: 1,
					message: "Picks cleared",
					action: { label: "Undo", run: () => {} },
				}}
			/>,
		);

		expect(markup).toContain('<button type="button">Undo</button>');
		expect(markup).not.toContain("autofocus");
		expect(markup).not.toContain('tabindex="-1"');
	});
});
