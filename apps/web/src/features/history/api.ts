import { geo } from "@tokuly/api";
import { api } from "#/lib/api";

export { historyKeys } from "@tokuly/core/history";

export const historyClient = () => api().service(geo.HistoryService);
