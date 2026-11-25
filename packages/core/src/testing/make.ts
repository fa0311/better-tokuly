import {
  create,
  type DescMessage,
  type MessageInitShape,
  type MessageShape,
} from "@bufbuild/protobuf";

/**
 * Builds a message for a unit test from the fields every case shares (`base`)
 * and the ones a case sets itself (`extra`, which wins).
 */
export function make<Schema extends DescMessage>(
  schema: Schema,
  base: MessageInitShape<Schema>,
  extra?: MessageInitShape<Schema>,
): MessageShape<Schema> {
  return create(schema, { ...(base as object), ...(extra as object) } as MessageInitShape<Schema>);
}
