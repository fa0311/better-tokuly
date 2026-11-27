import { geo } from "@tokuly/api";
import { api } from "#/lib/api";

export { footprintKeys, regionName } from "@tokuly/core/footprints";

export const footprintClient = () => api().service(geo.FootprintService);
