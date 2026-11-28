import { moments } from "@tokuly/api";
import { api } from "#/lib/api";

export const momentKeys = { today: ["moments", "today"] as const };
export const momentClient = () => api().service(moments.MomentService);
