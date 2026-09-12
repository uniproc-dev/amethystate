import { defineRouteMiddleware } from '@astrojs/starlight/route-data';

const base = import.meta.env.BASE_URL.replace(/\/$/, '');

export const onRequest = defineRouteMiddleware(({ locals }) => {
	const { locale } = locals.starlightRoute;
	locals.starlightRoute.siteTitleHref = locale
		? `${base}/${locale}/introduction/`
		: `${base}/introduction/`;
});
