KUBE_CONTEXT ?= docker-desktop
NAMESPACE ?= danske-spil
IMAGE_TAG ?= $(shell date +%Y%m%d%H%M%S)
GAMBLER_IMAGE ?= danske-spil-gambler:$(IMAGE_TAG)
RESULT_AGENT_IMAGE ?= $(GAMBLER_IMAGE)
BUILD_PROFILE ?= k8s-dev
DEPLOY_SCOPE ?= auto
GAMBLER_API ?= http://127.0.0.1:18083
METRICS_NAMESPACE ?= kube-system
METRICS_RELEASE ?= metrics-server
METRICS_CHART ?= metrics-server/metrics-server
METRICS_REPO_NAME ?= metrics-server
METRICS_REPO_URL ?= https://kubernetes-sigs.github.io/metrics-server/

.PHONY: account-history-agent-dry-run account-history-agent-fixture-dry-run account-history-agent-test docker-build docker-build-release k8s-deploy k8s-deploy-app k8s-deploy-full k8s-status metrics-api-install metrics-api-status metrics-api-top

account-history-agent-dry-run:
	rtk python3 scripts/account_history_agent.py --api $(GAMBLER_API) --dry-run

account-history-agent-fixture-dry-run:
	rtk python3 scripts/account_history_agent.py \
		--requests-json tests/fixtures/account_history_requests.json \
		--history-text-file tests/fixtures/account_history_text.txt \
		--context-radius 0 \
		--dry-run

account-history-agent-test:
	rtk python3 -m unittest tests.test_account_history_agent

docker-build:
	rtk docker build --build-arg BUILD_PROFILE=$(BUILD_PROFILE) -t $(GAMBLER_IMAGE) .
	if [ "$(RESULT_AGENT_IMAGE)" != "$(GAMBLER_IMAGE)" ]; then rtk docker tag $(GAMBLER_IMAGE) $(RESULT_AGENT_IMAGE); fi

docker-build-release:
	rtk make docker-build BUILD_PROFILE=release

k8s-deploy:
	KUBE_CONTEXT=$(KUBE_CONTEXT) NAMESPACE=$(NAMESPACE) IMAGE=$(GAMBLER_IMAGE) RESULT_AGENT_IMAGE=$(RESULT_AGENT_IMAGE) BUILD_PROFILE=$(BUILD_PROFILE) DEPLOY_SCOPE=$(DEPLOY_SCOPE) rtk bash scripts/deploy_local_k8s.sh

k8s-deploy-app:
	rtk make k8s-deploy DEPLOY_SCOPE=app

k8s-deploy-full:
	rtk make k8s-deploy DEPLOY_SCOPE=full

k8s-status:
	rtk kubectl --context $(KUBE_CONTEXT) -n $(NAMESPACE) get pods,deploy,svc,cluster

metrics-api-install:
	rtk helm repo add $(METRICS_REPO_NAME) $(METRICS_REPO_URL)
	rtk helm repo update $(METRICS_REPO_NAME)
	rtk helm upgrade --install $(METRICS_RELEASE) $(METRICS_CHART) \
		--kube-context $(KUBE_CONTEXT) \
		--namespace $(METRICS_NAMESPACE) \
		--set 'args[0]=--kubelet-insecure-tls' \
		--set 'args[1]=--kubelet-preferred-address-types=InternalIP\,Hostname\,ExternalIP'
	rtk kubectl --context $(KUBE_CONTEXT) -n $(METRICS_NAMESPACE) rollout status deployment/$(METRICS_RELEASE) --timeout=120s

metrics-api-status:
	rtk kubectl --context $(KUBE_CONTEXT) -n $(METRICS_NAMESPACE) get deployment $(METRICS_RELEASE)
	rtk kubectl --context $(KUBE_CONTEXT) get apiservice v1beta1.metrics.k8s.io

metrics-api-top:
	rtk kubectl --context $(KUBE_CONTEXT) -n danske-spil top pods
