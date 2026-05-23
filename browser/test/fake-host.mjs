/**
 * Fake Host for M40.3 Plugin Core testing
 *
 * 提供最小 host mock，记录所有 emit 事件和 render 调用。
 */

export function createFakeHost() {
  const events = [];
  const renderCalls = [];

  const host = {
    events,
    renderCalls,

    emit(eventName, payload) {
      events.push({ eventName, payload });
    },

    renderAnalysis(result) {
      renderCalls.push({ type: "renderAnalysis", result });
    },

    renderStatus(status) {
      renderCalls.push({ type: "renderStatus", status });
    },

    renderError(errorEnvelope) {
      renderCalls.push({ type: "renderError", errorEnvelope });
    },

    getEvents(name) {
      return events.filter((e) => e.eventName === name);
    },
  };

  return host;
}
