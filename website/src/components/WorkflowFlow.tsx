import { useEffect, useMemo, useState } from 'react';
import {
  Background,
  BackgroundVariant,
  Handle,
  MarkerType,
  Position,
  ReactFlow,
  type Edge,
  type Node,
  type NodeProps,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import './WorkflowFlow.css';

type WorkflowNodeData = {
  title: string;
  kind: string;
  detail: string;
  tone: 'input' | 'api' | 'function' | 'agent' | 'verify' | 'email';
  targetPosition?: Position;
  sourcePosition?: Position;
  hasTarget?: boolean;
  hasSource?: boolean;
};

type GroupNodeData = {
  label: string;
  detail: string;
  tone: 'orchestrator' | 'worker';
  targetPosition?: Position;
  sourcePosition?: Position;
  hasTarget?: boolean;
  hasSource?: boolean;
};

type FlowNodeData = WorkflowNodeData | GroupNodeData;

const steps = [
  'input',
  'stock-data',
  'summarize',
  'verify',
  'email',
] as const;

const nodeData: Array<Node<FlowNodeData>> = [
  {
    id: 'orchestrator',
    type: 'groupNode',
    position: { x: 24, y: 70 },
    style: { width: 230, height: 210 },
    data: {
      label: 'Orchestrator',
      detail: 'Captures workflow trigger input makes tasks available for workers',
      tone: 'orchestrator',
      sourcePosition: Position.Top,
      hasTarget: false,
    },
  },
  {
    id: 'worker',
    type: 'groupNode',
    position: { x: 300, y: 30 },
    style: { width: 600, height: 340 },
    data: {
      label: 'Remote Worker',
      detail: 'Polls orchestrator for work, executes tasks and passes results to next task in the chain',
      tone: 'worker',
      targetPosition: Position.Top,
      hasSource: false,
    },
  },
  {
    id: 'input',
    type: 'workflow',
    parentId: 'orchestrator',
    extent: 'parent',
    position: { x: 25, y: 72 },
    data: {
      title: 'User input',
      kind: 'Trigger: HTTP API',
      detail: 'Ticker symbols',
      tone: 'input',
      hasTarget: false,
      hasSource: false,
    },
  },
  {
    id: 'stock-data',
    type: 'workflow',
    parentId: 'worker',
    extent: 'parent',
    position: { x: 38, y: 74 },
    data: {
      title: 'Pull stock data',
      kind: 'API Call Tasks',
      detail: 'Fetch quote and price history',
      tone: 'api',
      hasTarget: false,
      hasSource: true,
    },
  },
  {
    id: 'summarize',
    type: 'workflow',
    parentId: 'worker',
    extent: 'parent',
    position: { x: 285, y: 74 },
    data: {
      title: 'Summarize data',
      kind: 'Agent Task',
      detail: 'Agent turns data into insight',
      tone: 'agent',
    },
  },
  {
    id: 'verify',
    type: 'workflow',
    parentId: 'worker',
    extent: 'parent',
    position: { x: 285, y: 214 },
    data: {
      title: 'Verify summary',
      kind: 'Agent Task',
      detail: 'Another agent checks expected information',
      tone: 'verify',
      targetPosition: Position.Right,
      sourcePosition: Position.Left,
    },
  },
  {
    id: 'email',
    type: 'workflow',
    parentId: 'worker',
    extent: 'parent',
    position: { x: 38, y: 214 },
    data: {
      title: 'Send email',
      kind: 'Function Task',
      detail: 'Delivers the verified report',
      tone: 'email',
      targetPosition: Position.Right,
      hasSource: false,
    },
  },
];

const edgeData: Edge[] = [
  {
    id: 'orchestrator-worker',
    source: 'orchestrator',
    sourceHandle: 'source-top',
    target: 'worker',
    targetHandle: 'target-top',
    type: 'smoothstep',
    markerEnd: { type: MarkerType.ArrowClosed },
  },
  {
    id: 'stock-data-summarize',
    source: 'stock-data',
    target: 'summarize',
    type: 'smoothstep',
    markerEnd: { type: MarkerType.ArrowClosed },
  },
  {
    id: 'summarize-verify',
    source: 'summarize',
    sourceHandle: 'source-right',
    target: 'verify',
    targetHandle: 'target-right',
    type: 'smoothstep',
    markerEnd: { type: MarkerType.ArrowClosed },
  },
  {
    id: 'verify-email',
    source: 'verify',
    sourceHandle: 'source-left',
    target: 'email',
    targetHandle: 'target-right',
    type: 'smoothstep',
    markerEnd: { type: MarkerType.ArrowClosed },
  },
];

function GroupNode({ data }: NodeProps<Node<GroupNodeData>>) {
  const targetPosition = data.targetPosition ?? Position.Left;
  const sourcePosition = data.sourcePosition ?? Position.Right;
  const hasTarget = data.hasTarget ?? true;
  const hasSource = data.hasSource ?? true;

  return (
    <div className={`workflow-group workflow-group--${data.tone}`}>
      {hasTarget ? (
        <Handle
          id={`target-${targetPosition.toLowerCase()}`}
          className="workflow-group-handle"
          type="target"
          position={targetPosition}
          isConnectable={false}
        />
      ) : null}
      {hasSource ? (
        <Handle
          id={`source-${sourcePosition.toLowerCase()}`}
          className="workflow-group-handle"
          type="source"
          position={sourcePosition}
          isConnectable={false}
        />
      ) : null}
      <div className="workflow-group__label">{data.label}</div>
      <div className="workflow-group__detail">{data.detail}</div>
    </div>
  );
}

function WorkflowNode({ data, selected }: NodeProps<Node<WorkflowNodeData>>) {
  const targetPosition = data.targetPosition ?? Position.Left;
  const sourcePosition = data.sourcePosition ?? Position.Right;
  const hasTarget = data.hasTarget ?? true;
  const hasSource = data.hasSource ?? true;

  return (
    <div className={`workflow-node workflow-node--${data.tone} ${selected ? 'is-active' : ''}`}>
      {hasTarget ? (
        <Handle
          id={`target-${targetPosition.toLowerCase()}`}
          className="workflow-handle"
          type="target"
          position={targetPosition}
          isConnectable={false}
        />
      ) : null}
      {hasSource ? (
        <Handle
          id={`source-${sourcePosition.toLowerCase()}`}
          className="workflow-handle"
          type="source"
          position={sourcePosition}
          isConnectable={false}
        />
      ) : null}
      <div className="workflow-node__meta">{data.kind}</div>
      <div className="workflow-node__title">{data.title}</div>
      <div className="workflow-node__detail">{data.detail}</div>
    </div>
  );
}

const nodeTypes = {
  groupNode: GroupNode,
  workflow: WorkflowNode,
};

export default function WorkflowFlow() {
  const [activeStep, setActiveStep] = useState(0);

  useEffect(() => {
    const timer = window.setInterval(() => {
      setActiveStep((current) => (current + 1) % steps.length);
    }, 1450);

    return () => window.clearInterval(timer);
  }, []);

  const activeNodeId = steps[activeStep];
  const activeEdgeId =
    activeStep === steps.length - 1
      ? undefined
      : activeStep === 0
        ? 'orchestrator-worker'
        : `${steps[activeStep]}-${steps[activeStep + 1]}`;

  const nodes = useMemo(
    () =>
      nodeData.map((node) => ({
        ...node,
        draggable: false,
        selectable: false,
        selected: node.id === activeNodeId,
      })),
    [activeNodeId],
  );

  const edges = useMemo(
    () =>
      edgeData.map((edge) => ({
        ...edge,
        animated: edge.id === activeEdgeId,
        className: edge.id === activeEdgeId ? 'workflow-edge is-active' : 'workflow-edge',
        focusable: false,
        selectable: false,
      })),
    [activeEdgeId],
  );

  return (
    <div className="workflow-flow" aria-label="Stock report workflow data flow">
      <ReactFlow
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        nodesDraggable={false}
        nodesConnectable={false}
        nodesFocusable={false}
        edgesFocusable={false}
        elementsSelectable={false}
        panOnDrag={false}
        panOnScroll={false}
        zoomOnDoubleClick={false}
        zoomOnPinch={false}
        zoomOnScroll={false}
        preventScrolling={false}
        fitView
        fitViewOptions={{ padding: 0.12 }}
        minZoom={0.5}
        maxZoom={1.1}
        proOptions={{ hideAttribution: true }}
      >
        <Background color="#d8e1eb" gap={22} size={1} variant={BackgroundVariant.Dots} />
      </ReactFlow>
    </div>
  );
}
