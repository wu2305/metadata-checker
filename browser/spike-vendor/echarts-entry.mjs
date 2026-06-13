import * as echarts from "echarts/core";
import { GraphChart } from "echarts/charts";
import { CanvasRenderer } from "echarts/renderers";
import { TooltipComponent } from "echarts/components";

echarts.use([GraphChart, CanvasRenderer, TooltipComponent]);

export { echarts };
export default echarts;
