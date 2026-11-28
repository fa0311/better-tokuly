import { useQuery } from "@tanstack/react-query";
import { moments } from "@tokuly/api";
import { momentKeys, momentQueries } from "@tokuly/core/moments";
import { api } from "#/lib/api";

export const momentClient = () => api().service(moments.MomentService);

export { momentKeys };

export function useMomentToday() {
  return useQuery(momentQueries.today(momentClient));
}
