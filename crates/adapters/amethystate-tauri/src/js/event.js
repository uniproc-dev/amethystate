function internals() {
	return window.__TAURI_INTERNALS__
}

async function emit(event, payload) {
	await internals().invoke('plugin:event|emit', { event, payload })
}

async function emitTo(target, event, payload) {
	const to = typeof target === 'string' ? { kind: 'AnyLabel', label: target } : target
	await internals().invoke('plugin:event|emit_to', { target: to, event, payload })
}

async function listen(event, handler, options) {
	const target =
		typeof options?.target === 'string'
			? { kind: 'AnyLabel', label: options.target }
			: (options?.target ?? { kind: 'Any' })

	const eventId = await internals().invoke('plugin:event|listen', {
		event,
		target,
		handler: internals().transformCallback(handler),
	})

	return async () => {
		window.__TAURI_EVENT_PLUGIN_INTERNALS__?.unregisterListener(event, eventId)
		await internals().invoke('plugin:event|unlisten', { event, eventId })
	}
}

async function once(event, handler, options) {
	const unlisten = await listen(
		event,
		(received) => {
			void unlisten()
			handler(received)
		},
		options,
	)
	return unlisten
}

export { emit, emitTo, listen, once }
